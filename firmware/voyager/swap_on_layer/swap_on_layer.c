// SPDX-License-Identifier: GPL-2.0-or-later
//
// Mirrors the whole keyboard while layer SWAP_ON_LAYER is active, with QMK's
// Swap Hands: key positions are swapped before any layer lookup, so every
// layer, hold-tap and layer key is reflected, not one hand-copied layer.
// Whatever turns the layer on starts it: a key holding the layer, or the foot
// pedal through ZSA's Oryx raw-HID protocol (SET_LAYER).
//
// The layer itself must be fully transparent so the layers below show
// through; the firmware build refuses one that is not.

#include QMK_KEYBOARD_H

ASSERT_COMMUNITY_MODULES_MIN_API_VERSION(1, 1, 0);

#ifndef SWAP_ON_LAYER
#    error "Define SWAP_ON_LAYER: the layer that mirrors the keyboard while active."
#endif

layer_state_t layer_state_set_swap_on_layer(layer_state_t state) {
    state = layer_state_set_swap_on_layer_kb(state);
    if (layer_state_cmp(state, SWAP_ON_LAYER)) {
        swap_hands_on();
    } else {
        swap_hands_off();
    }
    return state;
}
