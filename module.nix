{ config, lib, pkgs, ... }:

let
  cfg = config.services.ddcstuff;
in {
  options.services.ddcstuff = {
    enable = lib.mkEnableOption "ddcd DDC brightness daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.ddcstuff;
      description = "The ddcstuff package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Ensure the i2c group exists for DDC/CI access.
    users.groups.i2c = {};

    # Grant group-read/write on all i2c devices so members of 'i2c' can use DDC.
    services.udev.extraRules = ''
      KERNEL=="i2c-[0-9]*", GROUP="i2c", MODE="0660"
    '';

    # Run ddcd as a systemd user service that starts with the graphical session.
    systemd.user.services.ddcd = {
      enable = true;
      description = "DDC brightness daemon";
      partOf = [ "graphical-session.target" ];
      wantedBy = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/ddcd";
        Restart = "on-failure";
        RestartSec = "3s";
      };
    };
  };
}
