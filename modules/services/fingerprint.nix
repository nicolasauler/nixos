# Fingerprint reader scoped to one job: unlocking 1Password through polkit, so a
# passkey prompt in Firefox is a touch of the sensor. Nothing else leaves the
# keyboard.
#
# `services.fprintd.enable` flips `fprintAuth` on for EVERY PAM service
# (nixos/modules/security/pam.nix: `default = config.services.fprintd.enable`), and
# pam_fprintd sits first as `sufficient`: the prompt waits for a finger and only
# offers the password once fprintd gives up (its timeout, or three failed scans).
# For sudo, su, tty login, hyprlock that is exactly the hands-off-keyboard
# behaviour we don't want. So fprintAuth is re-defaulted to false for all services
# through the submodule type -- a PAM service that appears in a later nixpkgs stays
# keyboard-only too -- and turned back on for polkit-1 alone (1Password's "Unlock
# using system authentication" is a polkit action). tests/fingerprint-pam.nix proves
# both halves against libfprint's virtual reader.
#
# Hardware drivers stay with the host (hosts/precision: Dell ControlVault 3+ TOD
# module). Keeping this module driver-agnostic is what lets the VM test run stock
# fprintd, whose libfprint carries the virtual drivers; libfprint-tod does not.
{lib, ...}: {
  options.security.pam.services = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule {
      config.fprintAuth = lib.mkDefault false;
    });
  };

  config = {
    services.fprintd.enable = true;
    security.pam.services.polkit-1.fprintAuth = true;
  };
}
