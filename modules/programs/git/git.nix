{ lib, pkgs, ... }:
let
  # pubkey_files = lib.filterAttrs (k: v: v == "regular" && lib.hasSuffix ".pub" k) (builtins.readDir ./pubkeys);
  pubkeys_from_gh = builtins.readFile (pkgs.fetchurl {
    url = "https://github.com/nicolasauler.keys";
    hash = "sha256-XkIqd9WE/WrGqOG2T5ifDy3LCSsKit6ZBdQ9SkIWGbw=";
  });
  pubkey_lines = lib.strings.splitString "\n" pubkeys_from_gh;
  addAsterisk = line: "* " + line;
  allowed_lines = map addAsterisk pubkey_lines;
in
{
  #home.file.".ssh/allowed_signers".text =
  #"* ${builtins.readFile ./pubkeys/desktop_ssh.pub}";
  #  builtins.concatStringsSep "\n" (lib.mapAttrsToList (name: _: builtins.readFile ./pubkeys/${name}) pubkey_files);
  home.file.".ssh/allowed_signers".text = builtins.concatStringsSep "\n" allowed_lines;

  home.file.".config/git/commit_template".source = ./commit_template;

  programs.git = {
    enable = true;
    userName = "Nicolas Auler";
    userEmail = "nicolasauler@usp.br";

    extraConfig = {
      commit = {
        template = "/home/nic/.config/git/commit_template";
        gpgsign = true;
      };
      gpg = {
        format = "ssh";
        ssh.allowedSignersfile = "/home/nic/.ssh/allowed_signers";
      };
      user.signingkey = "/home/nic/.ssh/id_ed25519.pub";

      merge = { tool = "nvimdiff4"; prompt = false; };
      mergeTool = {
        nvimdiff4 = {
          cmd = "nvim -d $LOCAL $BASE $REMOTE $MERGED -c '$wincmd w' -c 'wincmd J'";
        };
      };
      core.editor = "nvim";
      init.defaultBranch = "main";
    };

    # signing.signByDefault = false;

    # https://ryantm.github.io/nixpkgs/builders/fetchers/
    # https://discourse.nixos.org/t/builtins-fetchurl-vs-nixpkgs-fetchurl/24133
    # https://nixos.org/manual/nix/unstable/language/builtins#builtins-fetchurl

    # https://ryantm.github.io/nixpkgs/functions/library/strings/
    # https://nixos.org/manual/nix/unstable/language/builtins#builtins-concatStringsSep

    # https://discourse.nixos.org/t/how-to-add-multiple-ssh-keys-from-a-folder/24839/2
    # https://www.reddit.com/r/NixOS/comments/15tbgmw/question_how_to_load_a_text_file_from_a_url_and/

    # https://nixos.org/manual/nix/unstable/language/builtins#builtins-readDir
  };
}
