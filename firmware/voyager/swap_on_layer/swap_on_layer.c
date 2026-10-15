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
//
// Mirrored nav (optional, MIRRORED_NAV_LAYER and NAV_LAYER): Oryx's SET_LAYER
// runs layer_move(n), which leaves only layer n on, so the pedal cannot turn
// on SWAP_ON_LAYER and NAV_LAYER together. MIRRORED_NAV_LAYER is an alias that
// does: a state holding it is rewritten here to the same state without it and
// with NAV_LAYER and SWAP_ON_LAYER on, so layer_move(MIRRORED_NAV_LAYER) gives
// nav, mirrored. The alias layer never reaches layer_state and needs no keymap
// entry (the build refuses one). layer_state_set stores what the module hooks
// return, and this module runs first, so the Oryx module and the keymap see
// only the rewritten state.
//
// Turning it off: layer_off(MIRRORED_NAV_LAYER) clears a bit that is no longer
// set, and the hook sees only the new state, so it cannot tell which of
// NAV_LAYER and SWAP_ON_LAYER came from the alias. The sender must clear those
// two itself (layer_off of each, or layer_move to whichever layer stays on).

#include QMK_KEYBOARD_H

ASSERT_COMMUNITY_MODULES_MIN_API_VERSION(1, 1, 0);

#ifndef SWAP_ON_LAYER
#    error "Define SWAP_ON_LAYER: the layer that mirrors the keyboard while active."
#endif

#ifdef MIRRORED_NAV_LAYER
#    ifndef NAV_LAYER
#        error "MIRRORED_NAV_LAYER needs NAV_LAYER: the layer it turns on, mirrored."
#    endif
#    if MIRRORED_NAV_LAYER < 0 || MIRRORED_NAV_LAYER >= (1 << MAX_LAYER_BITS)
#        error "MIRRORED_NAV_LAYER does not fit layer_state_t (LAYER_STATE_8BIT: layers 0..7)."
#    endif
#endif

layer_state_t layer_state_set_swap_on_layer(layer_state_t state) {
#ifdef MIRRORED_NAV_LAYER
    if (layer_state_cmp(state, MIRRORED_NAV_LAYER)) {
        state = (state & ~((layer_state_t)1 << MIRRORED_NAV_LAYER)) | ((layer_state_t)1 << NAV_LAYER) | ((layer_state_t)1 << SWAP_ON_LAYER);
    }
#endif
    state = layer_state_set_swap_on_layer_kb(state);
    if (layer_state_cmp(state, SWAP_ON_LAYER)) {
        swap_hands_on();
    } else {
        swap_hands_off();
    }
    return state;
}
