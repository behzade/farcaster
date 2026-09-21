{ testers, farcaster, ... }:
testers.runNixOSTest {
  name = "farcaster-native-startup";
  nodes.machine = { pkgs, ... }: {
    virtualisation = {
      memorySize = 4096;
      cores = 2;
      graphics = false;
    };
    hardware.graphics.enable = true;
    services.dbus.enable = true;
    fonts.packages = [ pkgs.dejavu_fonts ];
    users.users.tester = {
      isNormalUser = true;
      extraGroups = [
        "video"
        "render"
      ];
    };
    environment.systemPackages = with pkgs; [
      farcaster
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
        "su - tester -c 'dbus-run-session -- bash ${../scripts}/probe-native-startup.sh "
        "/run/current-system/sw/bin/farcaster /tmp/probe-logs' "
        "> /tmp/probe-console.log 2>&1",
        timeout=180,
    )
    machine.copy_from_machine("/tmp/probe-console.log")
    machine.copy_from_machine("/tmp/probe-logs")
    assert status == 0, machine.succeed("cat /tmp/probe-console.log")
  '';
}
