---
name: frontend-design
description: Guidance for distinctive, intentional visual design when building new UI or reshaping an existing one. Helps with aesthetic direction, typography, and making choices that don't read as templated defaults.
license: Complete terms in LICENSE.txt
---

# Frontend Design

Give the work a visual identity specific to its subject. Make deliberate
choices about palette, typography, and layout, and take one aesthetic risk
that the brief can justify.

## Ground it in the subject

If the brief leaves the subject vague, choose a concrete subject, audience, and
single purpose before designing, then state that choice. Use what is known
about the user's preferences and project. Draw visual ideas from the subject's
materials, tools, artifacts, and language, and use real content throughout.

## Design principles

For web designs, treat the hero as a thesis. Open with the most characteristic
part of the subject: perhaps a headline, image, animation, demo, or interaction.
A large statistic and gradient accent are useful only when the content calls
for them.

Typography carries the personality of the page. Pair the display and body faces deliberately, not the same families you would reach for on any other project, and set a clear type scale with intentional weights, widths, and spacing. Make the type treatment itself a memorable part of the design, not a neutral delivery vehicle for the content.

Structure is information. Numbering, dividers, labels, and other devices should
encode something true about the content. Use numbered markers only for a real
sequence or timeline.

Use motion only where it serves the subject. One orchestrated moment usually
works better than scattered effects, and many designs need no animation at all.

Match complexity to the vision. Maximalist directions need elaborate execution; minimal directions need precision in spacing, type, and detail. Elegance is executing the chosen vision well.

Treat copy as part of the design. If the brief lacks real content, write
specific working copy rather than generic product language.

## Process: brainstorm, explore, plan, critique, build, critique again

Three common defaults are a warm cream page with serif display type and a
terracotta accent; a near-black page with one acid-bright accent; and a dense
broadsheet layout with hairline rules. They are valid when the subject calls for
them, but should not fill an unspecified part of the brief automatically.
Follow an explicit visual direction even when it uses one of these patterns.

Work in two passes. First, make a short design plan covering:

- Color: four to six named hex values.
- Type: distinct display and body roles, plus a utility face when needed.
- Layout: a concise description and small ASCII wireframes where comparison
  helps.
- Signature: one memorable element grounded in the brief.

Review the plan against the brief before building. Replace any choice that
could have been made for an unrelated page, and note what changed. Then derive
the implementation's color and type decisions from the revised plan.

Keep CSS specificity predictable. Classes can cancel each other unexpectedly,
especially around section spacing and element-specific overrides.

Do most planning and iteration before presenting a direction to the user.

## Restraint and self-critique

Spend boldness in one place. Keep the supporting design quiet and remove
decoration that does not serve the brief. Make the result responsive, preserve
visible keyboard focus, and respect reduced-motion settings. Critique the work
from screenshots when possible and remove one unnecessary element before
finishing.

## More on writing in design

Words appear in a design for one reason: to make it easier to understand, and therefore easier to use. They are design material, not decoration. Bring the same intentionality to copy that you would bring to spacing and color. Before writing anything, ask what the design needs to say, and how it can best be said to help the person navigate the experience.

Write from the end user's side of the screen. Name things by what people control and recognize, never by how the system is built. A person manages notifications, not webhook config. Describe what something does in plain terms rather than selling it. Being specific is always better than being clever.

Use active voice as default. A control should say exactly what happens when it's used: "Save changes," not "Submit." An action keeps the same name through the whole flow, so the button that says "Publish" produces a toast that says "Published." The vocabulary of an interface is the signposting for someone navigating the product. Cohesion and consistency are how people learn their way around.

Treat failure and emptiness as moments for direction, not mood. Explain what went wrong and how to fix it, in the interface's voice rather than a person's. Errors don't apologize, and they are never vague about what happened. An empty screen is an invitation to act.

Keep the register conversational and tuned: plain verbs, sentence case, no filler, with tone matched to the brand and the audience. Let each element do exactly one job. A label labels, an example demonstrates, and nothing quietly does double duty.
