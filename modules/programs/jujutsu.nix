{...}: {
  programs.jujutsu = {
    enable = true;

    settings = {
      user = {
        name = "Nicolas Auler";
        email = "nickvarauler@gmail.com";
      };

      ui.default-command = "log";

      signing = {
        behavior = "own";
        backend = "ssh";
        key = "/home/nic/.ssh/id_ed25519.pub";
        backends.ssh.allowed-signers = "/home/nic/.ssh/allowed_signers";
      };
    };
  };
}
