"""Foot pedal -> Voyager layers, and Shift.

The PCsensor 3-pedal footswitch (3553:b001) sends its factory keys a, b, c.
This grabs it exclusively, so those never reach applications, and makes each
pedal a hold action:

* layer pedals run `kontroll set-layer -i N` on press and switch back on
  release: to the most recently pressed layer pedal still held, else 0.
  kontroll goes through keymapp's API to the firmware (ORYX_SET_LAYER, which
  is layer_move), so these are the Voyager's own layers; nothing is remapped.
* the Shift pedal holds Shift on a uinput keyboard. Hyprland ORs held
  modifiers across keyboards, so it applies to keys typed on the Voyager.
"""

import errno
import fcntl
import os
import signal
import struct
import subprocess
import sys
import time

# linux/input-event-codes.h
EV_SYN, EV_KEY, SYN_REPORT = 0, 1, 0
KEY_A, KEY_B, KEY_C, KEY_LEFTSHIFT = 30, 48, 46, 42
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
EVIOCGRAB = 0x40044590
UI_SET_EVBIT = 0x40045564
UI_SET_KEYBIT = 0x40045565
UI_DEV_SETUP = 0x405C5503
UI_DEV_CREATE = 0x5501


def log(message):
    print(message, file=sys.stderr, flush=True)


def uinput_keyboard(keys):
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
    for key in keys:
        fcntl.ioctl(fd, UI_SET_KEYBIT, key)
    setup = struct.pack("HHHH80sI", BUS_VIRTUAL, 0, 0, 1, b"foot-pedal", 0)
    fcntl.ioctl(fd, UI_DEV_SETUP, setup)
    fcntl.ioctl(fd, UI_DEV_CREATE)
    return fd


def emit(out, key, value):
    os.write(
        out,
        EVENT.pack(0, 0, EV_KEY, key, value)
        + EVENT.pack(0, 0, EV_SYN, SYN_REPORT, 0),
    )


def set_layer(layer):
    started = time.monotonic()
    try:
        result = subprocess.run(
            ["kontroll", "set-layer", "-i", str(layer)],
            capture_output=True,
            text=True,
            timeout=5,
        )
    except subprocess.TimeoutExpired:
        log(f"set-layer {layer}: kontroll timed out")
        return
    ms = (time.monotonic() - started) * 1000
    if result.returncode:
        detail = (result.stderr or result.stdout).strip()
        log(f"set-layer {layer} failed after {ms:.0f} ms: {detail}")
    else:
        log(f"set-layer {layer} ({ms:.0f} ms)")


def run(pedal, out):
    held_layers = []  # layer pedals currently down, in press order
    held_keys = set()

    def layer():
        return PEDALS[held_layers[-1]][1] if held_layers else 0

    fcntl.ioctl(pedal, EVIOCGRAB, 1)
    try:
        while True:
            data = os.read(pedal, EVENT.size * 64)
            for offset in range(0, len(data), EVENT.size):
                _, _, kind, code, value = EVENT.unpack_from(data, offset)
                if kind != EV_KEY or value == 2 or code not in PEDALS:
                    continue
                action, target = PEDALS[code]
                if action == "key":
                    emit(out, target, value)
                    if value:
                        held_keys.add(target)
                    else:
                        held_keys.discard(target)
                    continue
                before = layer()
                if value and code not in held_layers:
                    held_layers.append(code)
                elif not value and code in held_layers:
                    held_layers.remove(code)
                if layer() != before:
                    set_layer(layer())
    finally:
        for key in held_keys:
            emit(out, key, 0)
        if held_layers:
            set_layer(0)


def main():
    # systemd stops us with SIGTERM: unwind, so a held pedal is released
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    out = uinput_keyboard(
        {target for action, target in PEDALS.values() if action == "key"}
    )
    while True:
        try:
            pedal = os.open(DEVICE, os.O_RDONLY)
        except FileNotFoundError:
            time.sleep(2)
            continue
        log("pedal connected")
        try:
            run(pedal, out)
        except OSError as error:
            if error.errno != errno.ENODEV:
                raise
            log("pedal disconnected")
        finally:
            os.close(pedal)


main()
