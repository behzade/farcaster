# Run with the selected channel's nixpkgs and the exact candidate AppImage.
{ nixpkgs, appimage }:
let
  pkgs = import (builtins.toPath nixpkgs) { system = "x86_64-linux"; };
  candidate = builtins.path {
    path = builtins.toPath appimage;
    name = "Farcaster.AppImage";
  };
  probe = pkgs.writeShellScriptBin "probe-farcaster-appimage" ''
    export FARCASTER_PROBE_APPIMAGE_RUNNER=${pkgs.appimage-run}/bin/appimage-run
    export FARCASTER_PROBE_HOST_WAYLAND=${pkgs.wayland}/lib/libwayland-client.so.0
    exec ${pkgs.bash}/bin/bash ${../scripts/probe-appimage-startup.sh} \
      ${candidate} /tmp/probe-logs
  '';
in
pkgs.testers.runNixOSTest {
  name = "farcaster-appimage-startup";

  nodes.machine = {
    virtualisation = {
      memorySize = 4096;
      cores = 2;
      diskSize = 4096;
      graphics = false;
    };
    hardware.graphics.enable = true;
    services.dbus.enable = true;
    programs.appimage.enable = true;
    fonts.packages = [ pkgs.dejavu_fonts ];
    users.users.tester = {
      isNormalUser = true;
      extraGroups = [ "video" "render" ];
    };
    environment.systemPackages = with pkgs; [
      probe
      bash
      coreutils
      gawk
      gnugrep
      findutils
      file
      binutils
      weston
      wayland-utils
      xorg-server
      dbus
    ];
    system.stateVersion = "26.05";
  };

  testScript = ''
    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("install -d -o tester -g users /tmp/probe-logs")
    status, _ = machine.execute(
        "timeout 180s su - tester -c "
        "'dbus-run-session -- probe-farcaster-appimage' "
        "> /tmp/probe-console.log 2>&1",
        timeout=240,
    )
    machine.copy_from_machine("/tmp/probe-console.log")
    machine.copy_from_machine("/tmp/probe-logs")
    assert status == 0, machine.succeed("cat /tmp/probe-console.log")
  '';
}
