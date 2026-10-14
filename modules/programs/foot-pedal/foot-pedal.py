"""Foot pedal -> Voyager layers, and Shift.

The PCsensor 3-pedal footswitch (3553:b001) sends its factory keys a, b, c.
This grabs it exclusively, so those never reach applications, and makes each
pedal a hold action:

* layer pedals ask keymapp's API to change the Voyager's own firmware layer:
  SetLayer on press, UnsetLayer on release, which firmware 25 runs as
  layer_move(N) and layer_off(N) (zsa/qmk_modules oryx/oryx.c,
  ORYX_SET_LAYER). Nothing on the host remaps Voyager keystrokes. A worker
  thread owns the API: it applies the latest wanted state and retries until
  keymapp confirms it, so a slow or absent keymapp never delays reading the
  pedal, and a release that failed is not forgotten.
* the Shift pedal holds Shift on a uinput keyboard, immediately. Hyprland
  ORs held modifiers across keyboards, so it applies to keys typed on the
  Voyager.
"""

import errno
import fcntl
import os
import signal
import struct
import sys
import threading
import time

import grpc

# linux/input-event-codes.h
EV_SYN, EV_KEY = 0, 1
SYN_REPORT, SYN_DROPPED = 0, 3
KEY_A, KEY_B, KEY_C, KEY_LEFTSHIFT = 30, 48, 46, 42
KEY_MAX = 0x2FF
BUS_VIRTUAL = 0x06

# Layer indices of Oryx layout JqpmQ ("Colemak dh nicc"). Reordering layers
# in Oryx means updating these.
MOUSE, MIRROR = 5, 6

PEDALS = {
    KEY_A: ("key", KEY_LEFTSHIFT),  # left
    KEY_B: ("layer", MIRROR),  # middle
    KEY_C: ("layer", MOUSE),  # right
}

DEVICE = "/dev/input/by-id/usb-PCsensor_FootSwitch-event-kbd"

EVENT = struct.Struct("llHHi")  # struct input_event on 64-bit
KEY_BYTES = (KEY_MAX + 8) // 8
EVIOCGRAB = 0x40044590
EVIOCGKEY = (2 << 30) | (KEY_BYTES << 16) | (ord("E") << 8) | 0x18
UI_SET_EVBIT = 0x40045564
UI_SET_KEYBIT = 0x40045565
UI_DEV_SETUP = 0x405C5503
UI_DEV_CREATE = 0x5501

RETRY_SECONDS = 1
UNKNOWN = "unknown"  # firmware layer after a failed or unconfirmed request


def log(message):
    print(message, file=sys.stderr, flush=True)


class Pedals:
    """Which pedals are down, and what that asks for. No I/O."""

    def __init__(self, pedals):
        self.pedals = pedals
        self.held = []  # pedal codes down, in press order

    def press(self, code):
        if code not in self.pedals or code in self.held:
            return []
        self.held.append(code)
        return self._key(code, 1)

    def release(self, code):
        if code not in self.held:
            return []
        self.held.remove(code)
        return self._key(code, 0)

    def sync(self, down):
        """Adopt the device's real key state: at open, after a drop."""
        changes = []
        for code in [code for code in self.held if code not in down]:
            changes += self.release(code)
        for code in sorted(down - set(self.held)):  # press order is lost
            changes += self.press(code)
        return changes

    def layer(self):
        """Layer of the latest layer pedal still down, else None."""
        for code in reversed(self.held):
            action, target = self.pedals[code]
            if action == "layer":
                return target
        return None

    def _key(self, code, value):
        action, target = self.pedals[code]
        return [(target, value)] if action == "key" else []


def plan(applied, wanted, owned):
    """keymapp calls that take the firmware from `applied` to `wanted`."""
    if wanted is not None:
        return [] if applied == wanted else [("set", wanted)]
    if applied is None:
        return []
    if applied == UNKNOWN:  # a failed call, or a previous run that died
        return [("unset", layer) for layer in owned]
    return [("unset", applied)]


def varint(value):
    out = bytearray()
    while True:
        byte, value = value & 0x7F, value >> 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def read_varint(data, i):
    value = shift = 0
    while True:
        byte = data[i]
        i += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, i
        shift += 7


# keymapp's proto (kontroll 1.0.1 proto/keymapp.proto, package api):
#   message SetLayerRequest { int32 layer = 1; }
#   message SetLayerReply { bool success = 1; }
# Single-field messages, so they are encoded here rather than generated.
def layer_request(layer):
    return b"\x08" + varint(layer) if layer else b""  # proto3 omits 0


def success_reply(data):
    i, success = 0, False
    while i < len(data):
        tag, i = read_varint(data, i)
        field, wire = tag >> 3, tag & 7
        if wire == 0:
            value, i = read_varint(data, i)
            if field == 1:
                success = value != 0
        elif wire == 2:
            size, i = read_varint(data, i)
            i += size
        elif wire in (1, 5):
            i += 8 if wire == 1 else 4
        else:
            raise ValueError(f"unexpected wire type {wire}")
    return success


class Keymapp:
    """keymapp's gRPC API on its Unix socket, the one kontroll uses."""

    def __init__(self):
        config = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser(
            "~/.config"
        )
        socket = os.path.join(config, ".keymapp", "keymapp.sock")
        channel = grpc.insecure_channel(
            "unix:" + socket,
            # a restarted keymapp is reachable again within seconds, not
            # after grpc's default reconnect backoff of up to two minutes
            options=[
                ("grpc.initial_reconnect_backoff_ms", 250),
                ("grpc.max_reconnect_backoff_ms", 2000),
            ],
        )
        self.calls = {
            name: channel.unary_unary(
                f"/api.KeyboardService/{rpc}",
                request_serializer=layer_request,
                response_deserializer=success_reply,
            )
            for name, rpc in (("set", "SetLayer"), ("unset", "UnsetLayer"))
        }

    def call(self, name, layer):
        """None on success, else why it failed."""
        try:
            success = self.calls[name](layer, timeout=1)
        except grpc.RpcError as error:
            return f"{error.code().name}: {error.details()}"
        return None if success else "refused, no keyboard connected?"


class LayerWorker(threading.Thread):
    """Owns keymapp: applies the latest wanted layer, retrying until done."""

    def __init__(self, keymapp, owned):
        super().__init__(daemon=True)
        self.keymapp = keymapp
        self.owned = owned
        self.cond = threading.Condition()
        self.wanted = None
        self.applied = UNKNOWN  # so the first pass clears a stale layer
        self.failing = False

    def want(self, layer):
        with self.cond:
            self.wanted = layer
            self.cond.notify_all()

    def settled(self, timeout):
        """Wait until keymapp has confirmed what is wanted."""
        deadline = time.monotonic() + timeout
        with self.cond:
            while self.applied != self.wanted:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return False
                self.cond.wait(remaining)
            return True

    def run(self):
        while True:
            with self.cond:
                while self.applied == self.wanted:
                    self.cond.wait()
                wanted, applied = self.wanted, self.applied
            error = None
            for name, layer in plan(applied, wanted, self.owned):
                error = self.keymapp.call(name, layer)
                if error:
                    break
            with self.cond:
                self.applied = UNKNOWN if error else wanted
                self.cond.notify_all()
                if error and self.wanted == wanted:
                    self.cond.wait(RETRY_SECONDS)  # sooner if pedals move
            if error and not self.failing:
                log(f"keymapp: {error}; retrying every {RETRY_SECONDS}s")
            elif not error and self.failing:
                log("keymapp: reachable again, layer applied")
            self.failing = bool(error)


def uinput_keyboard(keys):
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
    for key in keys:
        fcntl.ioctl(fd, UI_SET_KEYBIT, key)
    setup = struct.pack("HHHH80sI", BUS_VIRTUAL, 0, 0, 1, b"foot-pedal", 0)
    fcntl.ioctl(fd, UI_DEV_SETUP, setup)
    fcntl.ioctl(fd, UI_DEV_CREATE)
    return fd


def emit(out, changes):
    for key, value in changes:
        os.write(
            out,
            EVENT.pack(0, 0, EV_KEY, key, value)
            + EVENT.pack(0, 0, EV_SYN, SYN_REPORT, 0),
        )


def held_now(pedal, codes):
    state = bytearray(KEY_BYTES)
    fcntl.ioctl(pedal, EVIOCGKEY, state)
    return {code for code in codes if state[code // 8] >> (code % 8) & 1}


def run(pedal, out, pedals, worker):
    def act(changes):
        emit(out, changes)
        worker.want(pedals.layer())

    fcntl.ioctl(pedal, EVIOCGRAB, 1)
    act(pedals.sync(held_now(pedal, PEDALS)))  # a pedal may already be down
    dropped = False
    while True:
        data = os.read(pedal, EVENT.size * 64)
        for offset in range(0, len(data), EVENT.size):
            _, _, kind, code, value = EVENT.unpack_from(data, offset)
            if kind == EV_SYN and code == SYN_DROPPED:
                dropped = True  # the queue overflowed: events were lost
            elif dropped:
                if kind == EV_SYN and code == SYN_REPORT:
                    dropped = False
                    act(pedals.sync(held_now(pedal, PEDALS)))
            elif kind == EV_KEY and value != 2:
                act(pedals.press(code) if value else pedals.release(code))


def main():
    # systemd stops us with SIGTERM: unwind, so held pedals are released
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    pedals = Pedals(PEDALS)
    owned = [target for action, target in PEDALS.values() if action == "layer"]
    worker = LayerWorker(Keymapp(), owned)
    worker.start()
    keys = {target for action, target in PEDALS.values() if action == "key"}
    try:
        out = uinput_keyboard(keys)
    except OSError as error:
        log(f"cannot create the Shift keyboard on /dev/uinput: {error}")
        raise SystemExit(1)
    try:
        while True:
            try:
                pedal = os.open(DEVICE, os.O_RDONLY)
            except FileNotFoundError:
                time.sleep(2)
                continue
            log("pedal connected")
            try:
                run(pedal, out, pedals, worker)
            except OSError as error:
                if error.errno != errno.ENODEV:
                    log(f"pedal: {error}")
                    raise
                log("pedal disconnected")
            finally:
                emit(out, pedals.sync(set()))
                worker.want(None)
                os.close(pedal)
    finally:
        worker.settled(timeout=2)


if __name__ == "__main__":
    main()
