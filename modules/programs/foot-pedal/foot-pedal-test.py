from foot_pedal import (
    KEY_A,
    KEY_B,
    KEY_C,
    KEY_LEFTSHIFT,
    MIRROR,
    MOUSE,
    PEDALS,
    UNKNOWN,
    Pedals,
    layer_request,
    plan,
    success_reply,
)

SHIFT, MIDDLE, RIGHT = KEY_A, KEY_B, KEY_C
LSHIFT_DOWN, LSHIFT_UP = (KEY_LEFTSHIFT, 1), (KEY_LEFTSHIFT, 0)

# Two layer pedals: the later press wins, releasing it hands back.
pedals = Pedals(PEDALS)
assert pedals.press(MIDDLE) == [] and pedals.layer() == MIRROR
assert pedals.press(RIGHT) == [] and pedals.layer() == MOUSE
assert pedals.release(RIGHT) == [] and pedals.layer() == MIRROR
assert pedals.release(MIDDLE) == [] and pedals.layer() is None

# Releasing the earlier pedal first leaves the later one in charge.
pedals = Pedals(PEDALS)
pedals.press(MIDDLE)
pedals.press(RIGHT)
assert pedals.release(MIDDLE) == [] and pedals.layer() == MOUSE
assert pedals.release(RIGHT) == [] and pedals.layer() is None

# Shift is a key, independent of the layer pedals.
pedals = Pedals(PEDALS)
assert pedals.press(SHIFT) == [LSHIFT_DOWN] and pedals.layer() is None
assert pedals.press(MIDDLE) == [] and pedals.layer() == MIRROR
assert pedals.release(SHIFT) == [LSHIFT_UP] and pedals.layer() == MIRROR

# A press already seen, a release never seen, a key that is no pedal: no-ops.
pedals = Pedals(PEDALS)
pedals.press(SHIFT)
assert pedals.press(SHIFT) == []
assert pedals.release(RIGHT) == []
assert pedals.press(KEY_LEFTSHIFT) == [] and pedals.held == [SHIFT]

# After a queue overflow, the real key state wins: what is up is released,
# what is down is pressed.
pedals = Pedals(PEDALS)
pedals.press(SHIFT)
pedals.press(MIDDLE)
assert pedals.sync({RIGHT}) == [LSHIFT_UP] and pedals.layer() == MOUSE
assert pedals.sync(set()) == [] and pedals.layer() is None

# At open, pedals already down take effect.
pedals = Pedals(PEDALS)
assert pedals.sync({SHIFT, RIGHT}) == [LSHIFT_DOWN]
assert pedals.layer() == MOUSE

# Firmware calls. Release is UnsetLayer (layer_off), so layers turned on
# from the keyboard during the hold survive it; an unknown state (a failed
# call, a previous run that died mid-hold) clears every pedal layer.
OWNED = [MIRROR, MOUSE]
assert plan(UNKNOWN, None, OWNED) == [("unset", MIRROR), ("unset", MOUSE)]
assert plan(MIRROR, None, OWNED) == [("unset", MIRROR)]
assert plan(None, None, OWNED) == []
assert plan(UNKNOWN, MOUSE, OWNED) == [("set", MOUSE)]
assert plan(MIRROR, MOUSE, OWNED) == [("set", MOUSE)]
assert plan(MOUSE, MOUSE, OWNED) == []

# keymapp's wire format: proto3 omits a zero field; replies are read by field
# number, skipping fields we do not know.
assert layer_request(MIRROR) == b"\x08\x06"
assert layer_request(0) == b""
assert layer_request(300) == b"\x08\xac\x02"
assert success_reply(b"\x08\x01") is True
assert success_reply(b"") is False
assert success_reply(b"\x12\x02hi\x08\x01") is True
