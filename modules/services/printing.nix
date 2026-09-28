# Printing that runs only while something prints, to network printers that are
# found rather than configured.
#
# "On demand" takes three settings, and only the first is a nixpkgs default:
#   - startWhenNeeded (default true) socket-activates cupsd. From boot, systemd
#     alone holds /run/cups/cups.sock and localhost:631, and the first client
#     starts the daemon.
#   - webInterface = false is what lets it stop again. Under systemd, cupsd
#     arms its idle exit only while the web interface is off (cups 2.4,
#     scheduler/main.c: `!WebInterface` in the IdleExitTimeout condition), so
#     with the default it stays up from its first client until shutdown.
#   - PreserveJobFiles No, because a kept job file is a scheduled wake-up. Its
#     expiry (a day, by default) becomes JobHistoryUpdate (scheduler/job.c), so
#     select_timeout never reports "nothing to do", and the idle exit can't arm
#     for a day after each print.
# With all three, cupsd exits about a minute after its last client or job
# (IdleExitTimeout is 60 s; the test measures 70-77 s after a job completes), and
# the socket brings it back on the next one.
#
# cups-browsed stays off although its default follows services.avahi.enable.
# It is a boot-time client of cupsd, so it starts the daemon at boot (the test
# measures cupsd starting a second after it). Its CVE-2024-47176 listener,
# UDP 631, is not open by default in the 2.1.1 packaged here (measured), so
# that is no longer the reason.
#
# Discovery: Avahi finds printers that speak IPP Everywhere / AirPrint (most
# printers since ~2013), and CUPS drives them with no vendor driver, as temporary
# queues (`lpstat -e`, then `lp -d NAME file`), with no cups-browsed involved.
# nssmdns4 is what lets cupsd resolve a printer's .local name (nixpkgs#395253).
# Avahi has to be the host's only mDNS stack. systemd-resolved is a full mDNS
# responder and resolver by default (on precision, `resolvectl mdns` says yes
# globally and on every LAN link), and with both on UDP 5353 avahi-daemon logs
# "Detected another IPv4 mDNS stack running on this host. This makes mDNS
# unreliable": a unicast reply reaches only one of them. CUPS browses through
# libavahi-client, so resolved's mDNS goes off, and Avahi publishes this host's
# address so its .local name keeps answering. Avahi runs from boot; it is the one
# always-on piece.
#
# tests/printing-on-demand.nix proves all of this in a VM, with the three
# on-demand settings back at their defaults as the control.
#
# There is no web interface, so administer from the CLI; wheel may (SystemGroup).
# `lpadmin -p NAME -E -v URI -m everywhere` makes a permanent queue. A USB-only
# printer may need services.ipp-usb.enable or a package in
# services.printing.drivers.
{...}: {
  services.printing = {
    enable = true;
    webInterface = false;
    extraConf = ''
      PreserveJobFiles No
    '';
    browsed.enable = false;
  };
  services.avahi = {
    enable = true;
    nssmdns4 = true; # resolve the printers' .local names
    openFirewall = true; # mDNS discovery, UDP 5353 (also the default)
    # answer for this host's own .local name, as resolved did before
    publish = {
      enable = true;
      addresses = true;
    };
  };
  # one mDNS stack (see Discovery above); inert on a host without resolved
  services.resolved.settings.Resolve.MulticastDNS = false;
}
