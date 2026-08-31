{
  config,
  lib,
  pkgs,
  ...
}: let
  python = config.services.buildbot-nix.packages.python;
  policyPackage = python.pkgs.toPythonModule (pkgs.runCommand "buildbot-pr-policy" {
    nativeBuildInputs = [
      python
      (python.pkgs.toPythonModule config.services.buildbot-nix.packages.buildbot)
    ];
  } ''
    cp ${./buildbot-pr-policy.py} buildbot_pr_policy.py
    PYTHONPATH="$PWD''${PYTHONPATH:+:$PYTHONPATH}" \
      python ${./buildbot-pr-policy-test.py}
    install -Dm0444 buildbot_pr_policy.py \
      "$out/${python.sitePackages}/buildbot_pr_policy.py"
  '');
in {
  services.buildbot-master = {
    pythonPackages = _: [policyPackage];
    extraImports = lib.mkAfter ''
      from buildbot_pr_policy import WorkstationPolicyConfigurator
    '';
    configurators = lib.mkAfter [
      ''
        WorkstationPolicyConfigurator(
          pr_authors={"nicolasauler"},
          build_pushes=False,
          max_concurrent_nix_builds=1,
        )
      ''
    ];
  };
}
