# Behavioural proof of modules/services/printing.nix: a printer that announces
# itself over mDNS prints with no driver and no queue configured, and cupsd runs
# only while something needs it.
#
# Why a VM and not an eval assertion. Reading startWhenNeeded back proves a
# socket unit exists, not that cupsd ever leaves: the idle exit is decided at
# runtime by cupsd itself (scheduler/main.c), from the web interface setting and
# from whatever it has scheduled, such as a kept job file's expiry. Driverless
# printing is a chain nothing evaluates, either: Avahi has to be the only mDNS
# stack next to systemd-resolved, libcups has to list the printer as a temporary
# queue, cupsd has to create that queue for an ordinary user and resolve the
# printer's .local name through nss-mdns (nixpkgs#395253 is that step failing),
# and the job has to arrive.
#
# The printer is CUPS's own IPP Everywhere simulator, ippeveprinter, announced
# through Avahi on its own node; it keeps what it receives (-k), so the test sees
# the document arrive. It starts only after the first control, so nothing but its
# announcement can have told the laptop it exists, and /etc/hosts names the nodes
# but not their .local names. It serves ipps with a self-signed certificate, as
# real printers do, and CUPS takes that over plain ipp.
#
# The control node is the same module with its three on-demand settings back at
# their nixpkgs defaults. It is what makes "cupsd exited" mean something: without
# it, the exit could just as well be a crash, a unit timeout, or CUPS always
# behaving that way. Measured while writing this: the control's cupsd starts a
# second after cups-browsed, at boot, and never exits; the laptop's exits 70-77 s
# after the job completes, over two runs (IdleExitTimeout is 60 s).
#
# The laptops enable systemd-resolved because precision does. That gives them
# precision's NSS order (mdns4_minimal before resolve) and, until the module
# turned it off, precision's second mDNS stack: both laptops logged avahi-daemon's
# "another IPv4 mDNS stack" warning.
#
# `lpstat -r` is no proof that cupsd is back: it says "scheduler is running" as
# soon as systemd accepts the connection on the socket (measured: 0.02 s, while
# the unit was still starting). So the last subtest prints again.
#
# No private input and no credential: the users have no password, the printer
# takes jobs unauthenticated, and the only document is a generated test page, so
# this is safe for the public CI job.
{pkgs, ...}: let
  user = "nic";
  service = "No Quarter Test";
  # CUPS's temporary-queue name for that DNS-SD service name
  queue = "No_Quarter_Test";
  port = 8631;
  spool = "/var/lib/ippeveprinter";
  page = pkgs.runCommand "test-page.pdf" {nativeBuildInputs = [pkgs.ghostscript];} ''
    gs -q -dNOPAUSE -dBATCH -sDEVICE=pdfwrite -sOutputFile=$out \
      -c '/Helvetica findfont 24 scalefont setfont 72 720 moveto (No Quarter) show showpage'
  '';
  laptop = extra: {
    imports = [
      ../modules/services/printing.nix
      extra
    ];
    # precision's resolver, so NSS has precision's hosts order and resolved is
    # there to be a second mDNS stack
    services.resolved.enable = true;
    users.users.${user}.isNormalUser = true;
  };
in
  pkgs.testers.runNixOSTest {
    name = "printing-on-demand";

    nodes = {
      laptop = laptop {};

      # The control: the same module, its three on-demand settings back at the
      # nixpkgs defaults.
      stock = laptop ({lib, ...}: {
        services.printing = {
          webInterface = lib.mkForce true;
          extraConf = lib.mkForce "";
          browsed.enable = lib.mkForce true;
        };
      });

      printer = {pkgs, ...}: {
        services.avahi = {
          enable = true;
          publish = {
            enable = true;
            userServices = true;
          };
        };
        networking.firewall.allowedTCPPorts = [port];
        # Not wanted by any target: the test starts it after the first control.
        systemd.services.ippeveprinter = {
          after = ["avahi-daemon.service"];
          requires = ["avahi-daemon.service"];
          serviceConfig = {
            # -K: somewhere writable for the self-signed certificate it serves
            # ipps with, which CUPS prefers when a printer offers both
            ExecStart = "${pkgs.cups}/bin/ippeveprinter -v -p ${toString port} -d ${spool} -k -K ${spool}/ssl '${service}'";
            StateDirectory = ["ippeveprinter" "ippeveprinter/ssl"];
          };
        };
      };
    };

    testScript = ''
      import time
      from datetime import timedelta

      start_all()
      for m in (printer, laptop, stock):
          m.wait_for_unit("multi-user.target")


      def running(m):
          return m.execute("systemctl is-active --quiet cups.service")[0] == 0


      with subtest("from boot, cupsd is not running: only its socket and Avahi are"):
          laptop.succeed("systemctl is-active cups.socket avahi-daemon.service")
          assert not running(laptop), "cupsd is up with nothing printing"
          laptop.fail("systemctl cat cups-browsed.service")

      with subtest("control: with the defaults, cupsd is up before anything printed"):
          stock.wait_until_succeeds("systemctl is-active --quiet cups.service", timeout=timedelta(seconds=60))

      with subtest("Avahi is the only mDNS stack, and still answers for the laptop's name"):
          listeners = laptop.succeed("ss -Hlunp 'sport = :5353'").strip().splitlines()
          assert listeners and all('"avahi-daemon"' in l for l in listeners), listeners
          laptop.fail("journalctl -u avahi-daemon.service | grep -q 'another IPv4 mDNS stack'")
          printer.wait_until_succeeds("avahi-resolve-host-name -4 laptop.local | grep -q '^laptop.local'", timeout=timedelta(seconds=30))

      with subtest("control: before any printer announces itself, there is nothing to print to"):
          assert "${queue}" not in laptop.succeed("lpstat -e || true")
          laptop.fail("runuser -u ${user} -- lp -d ${queue} ${page}")

      with subtest("a printer announced over mDNS appears as a temporary queue"):
          printer.systemctl("start ippeveprinter.service")
          printer.wait_for_open_port(${toString port})
          laptop.wait_until_succeeds("lpstat -e | grep -qx ${queue}", timeout=timedelta(seconds=60))

      with subtest("an ordinary user prints to it, with no driver and no queue configured"):
          laptop.succeed("runuser -u ${user} -- lp -d ${queue} ${page}")
          # job 1's document, in whichever format CUPS picked from the printer's
          # list (it rasterises to image/urf here), then the laptop's side of it
          printer.wait_until_succeeds("test -s ${spool}/1-*", timeout=timedelta(seconds=120))
          laptop.wait_until_succeeds("lpstat -W completed -o ${queue} | grep -q '^${queue}-1 '", timeout=timedelta(seconds=120))
          completed = time.monotonic()

      with subtest("cupsd exits by itself once idle"):
          laptop.wait_until_fails("systemctl is-active --quiet cups.service", timeout=timedelta(seconds=180))
          print(f"cupsd exited {time.monotonic() - completed:.0f} s after the job completed")
          laptop.succeed("journalctl -u cups.service | grep -q 'will restart on demand'")

      with subtest("control: with the defaults, cupsd is still up after the same idle time"):
          assert running(stock), "the control's cupsd exited too"
          stock.fail("journalctl -u cups.service | grep -q 'will restart on demand'")

      with subtest("the next print starts cupsd again, and prints"):
          laptop.succeed("systemctl is-active cups.socket")
          laptop.succeed("runuser -u ${user} -- lp -d ${queue} ${page}")
          laptop.wait_for_unit("cups.service")
          # the printer numbers its own jobs, so the second document is 2-*
          printer.wait_until_succeeds("test -s ${spool}/2-*", timeout=timedelta(seconds=120))
    '';
  }
