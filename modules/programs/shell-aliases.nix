# Shared by every host: `home.shellAliases` feeds both bash and nushell.
#
# Where an alias overlaps one a program module already generates, defer to the module
# instead of restating its flags -- see `ls`.
{...}: {
  home.shellAliases = {
    cd = "z";

    # Not `eza --icons`: the flag's value is optional, so a bare `--icons` swallows the
    # following path and `ls ~/Documents` fails. Plain `eza` chains through the alias
    # programs.eza generates, keeping icons/git configured in one place.
    ls = "eza";

    g = "git";
    n = "nvim";
    vi = "nvim .";
    cat = "bat";
    ps = "procs";
  };
}
