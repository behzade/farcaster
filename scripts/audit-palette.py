#!/usr/bin/env python3
"""Audit the built-in Farcaster CSS palette; requires sharkdp/pastel on PATH.

python3 scripts/audit-palette.py --baseline before.css --json /tmp/palette.json

Ratios use unrounded WCAG sRGB luminance, not Pastel's rounded display value.
Simulations and CIE76 distances are advisory, not a color-vision pass/fail test.
This checks opaque theme tokens, not rendered UI or full WCAG conformance.
"""

import argparse
from collections import Counter
import hashlib
import itertools
import json
import math
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / "assets/themes/farcaster.css"
SURFACES = ("canvas", "panel", "inspector", "composer", "surface", "highlight")
TEXT = ("text", "muted", "subtle", "accent", "accent_hover", "accent_active",
        "link", "code", "skill", "file", "warning", "error", "danger", "success",
        "indicator")
PALETTE = {*SURFACES, *TEXT, "border", "focus_border", "backdrop", "terminal_green"}
FILLS = ("accent", "accent_hover", "accent_active", "danger", "error",
         "warning", "success")


def read_colors(css):
    # This audit deliberately accepts only complete, opaque bundled palettes.
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    entries = re.findall(r"--([\w-]+)\s*:\s*([^;]+);", css)
    names = [name for name, _ in entries]
    if len(names) != len(set(names)):
        raise ValueError("duplicate theme token")
    colors = {}
    for name, value in entries:
        if value.startswith("#") and name != "backdrop":
            if not re.fullmatch(r"#[0-9a-fA-F]{6}", value):
                raise ValueError(f"{name}: expected an opaque six-digit color")
            colors[name] = value.lower()
    missing = PALETTE - {"backdrop", "terminal_green"} - colors.keys()
    if missing:
        raise ValueError(f"missing palette colors: {sorted(missing)}")
    # Saved themes from before the separate terminal token retain their green.
    colors.setdefault("terminal_green", colors["success"])
    return colors


def luminance(color):
    channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
              for v in channels]
    return sum(v * w for v, w in zip(linear, (0.2126, 0.7152, 0.0722)))


def contrast(a, b):
    low, high = sorted((luminance(a), luminance(b)))
    return (high + 0.05) / (low + 0.05)


def pairs(colors):
    for name in colors:
        if name in TEXT or name not in PALETTE:
            icon = name.startswith("icon-")
            group = "icon" if icon else "text" if name in TEXT else "syntax"
            for surface in SURFACES:
                yield group, name, surface, 3.0 if icon else 4.5
    for surface in SURFACES:
        yield "focus", "focus_border", surface, 3.0
    yield "terminal", "terminal_green", "canvas", 4.5
    for fill in FILLS:
        yield "button", "canvas", fill, 4.5


def checks(colors):
    rows = []
    for group, fg, bg, threshold in pairs(colors):
        ratio = contrast(colors[fg], colors[bg])
        rows.append(dict(group=group, foreground=fg, background=bg, ratio=ratio,
                         minimum=threshold, meets_target=ratio >= threshold))
    return rows


def pastel(*args, stdin=None):
    return subprocess.run(["pastel", *args], input=stdin, text=True,
                          capture_output=True, check=True).stdout


def simulate(colors, profile):
    values = list(colors.values())
    raw = (pastel("to-gray", *values) if profile == "gray"
           else pastel("colorblind", profile, *values))
    converted = pastel("format", "hex", stdin=raw).splitlines()
    if len(converted) != len(colors):
        raise ValueError("unexpected Pastel simulation output")
    return dict(zip(colors, converted))


def summary(rows):
    worst = min(rows, key=lambda row: row["ratio"] / row["minimum"])
    return dict(count=len(rows), below_target=sum(not r["meets_target"] for r in rows),
                groups=dict(Counter(r["group"] for r in rows)), worst=worst)


def status_distances(colors):
    names = ("success", "warning", "error")
    labs = pastel("format", "lab", *(colors[n] for n in names)).splitlines()
    vectors = [tuple(map(float, re.findall(r"-?\d+(?:\.\d+)?", lab))) for lab in labs]
    if len(vectors) != 3 or any(len(v) != 3 for v in vectors):
        raise ValueError("unexpected Pastel Lab output")
    return [{"pair": [names[a], names[b]], "delta_e_76": math.dist(vectors[a], vectors[b])}
            for a, b in itertools.combinations(range(3), 2)]


def audit(path):
    css = path.read_text()
    colors = read_colors(css)
    profiles = {"normal": colors}
    profiles.update({profile: simulate(colors, profile)
                     for profile in ("prot", "deuter", "trit", "gray")})
    results = {}
    for profile, values in profiles.items():
        rows = checks(values)
        results[profile] = dict(colors=values, summary=summary(rows), checks=rows,
                                status_distances=status_distances(values))
    # Low-contrast separators may only serve as decoration, not required boundaries.
    separators = {bg: contrast(colors["border"], colors[bg]) for bg in SURFACES}
    return dict(path=str(path), sha256=hashlib.sha256(css.encode()).hexdigest(),
                profiles=results, decorative_border_ratios=separators)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("palette", type=Path, nargs="?", default=DEFAULT)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--json", type=Path, help="write full pairs and simulation colors")
    args = parser.parse_args()
    report = dict(
        pastel=pastel("--version").strip(),
        method={
            "contrast": "WCAG 2.2 relative sRGB luminance; unrounded comparisons",
            "targets": "text/syntax/buttons 4.5:1; icons/focus/input outlines 3:1",
            "simulation": "Pastel prot/deuter/trit plus luminance-preserving grayscale; advisory",
            "status_distance": "CIE76 using Pastel's rounded Lab output; indicative, no pass threshold",
            "sources": [
                "https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html",
                "https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html",
                "https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html",
            ],
        },
        current=audit(args.palette),
    )
    if args.baseline:
        report["baseline"] = audit(args.baseline)
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    for label in ("baseline", "current"):
        if label not in report:
            continue
        print(f"{label}: {report[label]['path']}")
        for profile, result in report[label]["profiles"].items():
            s = result["summary"]
            w = s["worst"]
            print(f"  {profile:6}: {s['count'] - s['below_target']}/{s['count']} meet targets; "
                  f"worst relative to target: {w['foreground']}/{w['background']} "
                  f"{w['ratio']:.3f}:1 (target {w['minimum']}:1)")
    print("Simulated targets are advisory. Status colors still need shapes or labels.")
    print("Decorative borders, selection fills alone, inherited component colors, "
          "ANSI backgrounds, opacity, and live rendering need separate review.")
    return int(report["current"]["profiles"]["normal"]["summary"]["below_target"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())
