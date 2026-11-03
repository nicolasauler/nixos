-- Keeps a Bluetooth headset's input mute where every tool shows it
-- (pavucontrol, wpctl, talkd's bar indicator), and the mute you last set.
--
-- WirePlumber's Bluetooth input (bluez_input.<address>, a module-loopback
-- source) has two mutes: the device's Route for device 0, which every tool
-- uses when it exists, and the loopback node's own Props, which the tools fall
-- back to while the device publishes no such Route. bluez5 skips that Route
-- when the headset link joins after the device was created, while the link is
-- down, during an asynchronous codec switch and at disconnect. A mute set in
-- such a window stays in the node's Props, unseen and uncleared, once the Route
-- comes back. Separately, WirePlumber restores stored Route props on every
-- profile switch, deciding before it has seen the latest mute (it saves a
-- client's Route change at a lower event priority, and reads the device
-- through PipeWire's param cache, which can return the previous mute for a
-- while after a change), so a mute set just before a switch could be undone;
-- and bluez5 creates a profile's headset input unmuted, which WirePlumber does
-- not always correct.
--
-- This script remembers the mute you last set, per device and across
-- reconnects, reads and sets the device's routes on bluez5 directly (the
-- SpaDevice inside WirePlumber, not PipeWire's cache), and:
--   - when WirePlumber restores routes after a profile switch, sets the input
--     route itself, with the latest mute (a mute found on a route that only
--     now appears is kept);
--   - moves a mute held in the loopback's Props to the Route once the Route
--     exists; a mute that turns up there while the Route exists (any writer)
--     is moved too, with a warning;
--   - while you are muted and the real headset input (bluez_input.<address>.0)
--     does not show it yet, mutes the loopback as well, so nothing gets
--     through; it re-sends the Route mute if the input is still live 500 ms
--     later;
--   - when the Route disappears while the input stays, mutes or unmutes the
--     loopback's Props instead.
--   - while talkd (push-to-talk) owes a restore, remembers the mute it will
--     put back rather than F13's flip, so a headset that drops mid-press
--     comes back at your setting.
-- Left as is: a Route write that bluez5 rejects during an asynchronous codec
-- switch, and one that lands in the instant between this script reading the
-- route and setting it at a switch; both stay visible, so the bar shows what
-- the mic does.

local cutils = require ("common-utils")
local log = Log.open_topic ("s-bluez-loopback-mute")

-- device id within the card profile that Bluetooth inputs use
local INPUT_DEVICE = 0
-- how long our own write may take to show up before it is sent again
local ECHO_TIMEOUT_MSEC = 2000
-- how often a write that does not show up is sent again
local MAX_RETRIES = 3
-- how long a profile switch may take before its routes are re-applied
local TRANSITION_TIMEOUT_MSEC = 3000
-- how long a Route mute may take to reach the real headset input
local HEADSET_INPUT_MUTE_MSEC = 500

-- The bluez monitor and, per device.name, the id under which it manages the
-- device's SpaDevice. The SpaDevice is never kept (the sandbox has no
-- collectgarbage, and a kept one would outlive its removal); it is looked up
-- when needed.
local monitor = nil
local spa_ids = {}
-- per device.name, a count of SpaDevices created under it; a signal handler
-- acts only while its SpaDevice is the latest (sub-ids are reused)
local generations = {}
-- per bluez device with a loopback input, keyed by device.name
local devices = {}
local state = State ("bluez-loopback-mute")
local state_table = state:load ()

local function serial (object)
  return object.properties ["object.serial"]
end

local function spaDevice (name)
  local id = spa_ids [name]
  local spa = monitor and id and monitor:get_managed_object (id)
  if spa and spa.properties ["device.name"] == name then
    return spa
  end
  return nil
end

-- The device-0 input route among Route params (an iterator), as
-- { index, mute }, mute nil when the route carries no props; nil when there
-- is none.
local function findRoute (...)
  for p in ... do
    local route = cutils.parseParam (p, "Route")
    if route and route.device == INPUT_DEVICE and route.direction == "Input" then
      local props = route.props and route.props.properties or {}
      return { index = route.index, mute = props.mute }
    end
  end
  return nil
end

-- The route straight from bluez5, synchronously; second value false if the
-- device is gone.
local function readRoute (name)
  local spa = spaDevice (name)
  if not spa then
    return nil, false
  end
  return findRoute (spa:iterate_params ("Route")), true
end

local function readProfile (spa)
  local name = nil
  for p in spa:iterate_params ("Profile") do
    local profile = cutils.parseParam (p, "Profile")
    if profile then
      name = profile.name
    end
  end
  return name
end

-- A node's own mute, through PipeWire's cache. For the real headset input the
-- only writer is bluez5 (ObjectConfig from Route writes). A stale "live" only
-- keeps the loopback muted longer; a stale "muted" could unmute the loopback
-- while the input is live, which takes bluez5 making the input live without a
-- Route change: seen only for a new input, whose reading is new.
local function readPropsMute (node)
  if not node then
    return nil
  end
  for p in node:iterate_params ("Props") do
    local props = cutils.parseParam (p, "Props")
    if props and props.mute ~= nil then
      return props.mute
    end
  end
  return nil
end

local function saveState ()
  local ok, err = state:save (state_table)
  if not ok then
    log:warning ("could not save the remembered mutes: " .. tostring (err))
  end
end

-- The mute last set for a device: this script's state, else WirePlumber's
-- stored input route props (the first input route by name), else unmuted.
local function rememberedMute (device_name)
  local own = state_table [device_name]
  if own ~= nil then
    return own == "true"
  end
  local prefix = device_name .. ":input:"
  local routes = State ("default-routes"):load ()
  local keys = {}
  for key, _ in pairs (routes) do
    if key:sub (1, #prefix) == prefix then
      table.insert (keys, key)
    end
  end
  table.sort (keys)
  for _, key in ipairs (keys) do
    local json = Json.Raw (routes [key])
    if json and json:is_object () then
      local props = json:parse ()
      if props.mute ~= nil then
        return props.mute
      end
    end
  end
  return false
end

-- talkd (push-to-talk: F13 inverts the mute until released) publishes every
-- restore it owes, for an F13 press or for a mic that went away mid-press:
-- "default" metadata, subject 0, key "talkd.owed", { "<node.name>": <mute> }.
-- For a headset whose input is named there, the mute to remember is the one
-- talkd will put back, not the flip, so a headset that drops mid-press comes
-- back at your setting. The live mute is unaffected.
local talkd_owed = {}

local function remembered (d)
  local owed = d.input_name and talkd_owed [d.input_name]
  if owed ~= nil then
    return owed
  end
  if d.retained ~= nil then
    return d.retained
  end
  return d.intent
end

-- Save the mute to remember for this device, if it changed.
local function remember (d)
  local value = tostring (remembered (d))
  if state_table [d.name] ~= value then
    state_table [d.name] = value
    saveState ()
  end
end

-- Stop remembering the mute talkd put back over this script's own view; a
-- read of it still under way no longer counts.
local function release (d)
  d.retained = nil
  d.retire_reading = false
  d.retire_token = (d.retire_token or 0) + 1
end

local askRetired

local function setIntent (d, mute, why)
  if d.intent == mute then
    return
  end
  log:info (string.format ("%s: mute %s (%s)", d.name, tostring (mute), why))
  d.intent = mute
  -- a read for a done restore that failed is asked again at the next change
  if d.retained ~= nil and not d.retire_reading and d.node then
    askRetired (d)
  end
  remember (d)
end

local reconcile

-- Our own writes, so that their echoes are not taken for the user's. A write
-- that does not show up (bluez5 drops Route writes during a codec switch) is
-- sent again a few times.
local function expect (d, where, value)
  local key = where .. ":" .. tostring (value)
  if d.pending [key] then
    d.pending [key]:destroy ()
  end
  d.pending [key] = Core.timeout_add (ECHO_TIMEOUT_MSEC, function ()
    d.pending [key] = nil
    d.retries [where] = (d.retries [where] or 0) + 1
    if d.retries [where] > MAX_RETRIES then
      log:warning (string.format (
          "%s: setting the %s mute to %s keeps not showing up; giving up until it changes",
          d.name, where, tostring (value)))
    else
      log:info (string.format (
          "%s: setting the %s mute to %s did not show up; sending it again",
          d.name, where, tostring (value)))
      reconcile (d)
    end
    return false
  end)
end

local function consume (d, where, value)
  local key = where .. ":" .. tostring (value)
  if d.pending [key] then
    d.pending [key]:destroy ()
    d.pending [key] = nil
    d.retries [where] = 0
    return true
  end
  return false
end

local function pendingOn (d, where)
  return d.pending [where .. ":true"] ~= nil
      or d.pending [where .. ":false"] ~= nil
end

local function mayWrite (d, where)
  return not pendingOn (d, where) and (d.retries [where] or 0) <= MAX_RETRIES
end

-- Sets the input route on bluez5 directly; props (a Lua table of Route props,
-- arrays already as Pods) default to the mute alone. Only a write that
-- changes the mute is expected to echo.
local function setRoute (d, index, mute, props, save)
  local spa = spaDevice (d.name)
  if not spa then
    return
  end
  props = props or {}
  props.mute = mute
  table.insert (props, 1, "Spa:Pod:Object:Param:Props")
  table.insert (props, 2, "Route")
  if d.route and d.route.mute ~= mute then
    expect (d, "route", mute)
  end
  spa:set_param ("Route", Pod.Object {
    "Spa:Pod:Object:Param:Route", "Route",
    index = index,
    device = INPUT_DEVICE,
    props = Pod.Object (props),
    save = save,
  })
end

local function setPropsMute (d, mute)
  if d.props_mute ~= mute then
    expect (d, "props", mute)
  end
  d.node:set_param ("Props", Pod.Object {
    "Spa:Pod:Object:Param:Props", "Props",
    mute = mute,
  })
end

local function endTransition (d)
  if d.transition then
    d.transition:destroy ()
    d.transition = nil
  end
end

local function beginTransition (d)
  endTransition (d)
  d.generation = d.generation + 1
  d.transition = Core.timeout_add (TRANSITION_TIMEOUT_MSEC, function ()
    d.transition = nil
    log:info (d.name .. ": no route restore after the switch; taking the routes as they are")
    reconcile (d)
    return false
  end)
end

-- Mute the loopback too while you are muted and the real headset input does
-- not show it (a new input, or one bluez5 has not reached yet): runs also
-- during a switch. Re-sends the Route mute if the input is still live later.
local function protect (d)
  if not d.intent or not d.node or not d.real or readPropsMute (d.real) ~= false then
    return
  end
  if d.props_mute ~= true and mayWrite (d, "props") then
    setPropsMute (d, true)
  end
  if not d.checking_real and (d.real_fixes or 0) < MAX_RETRIES then
    d.checking_real = Core.timeout_add (HEADSET_INPUT_MUTE_MSEC, function ()
      d.checking_real = nil
      local now = readRoute (d.name)
      if devices [d.name] == d and d.intent and d.real and now and now.mute
          and readPropsMute (d.real) == false then
        d.real_fixes = (d.real_fixes or 0) + 1
        log:warning (string.format (
            "%s: the headset input was still live %d ms after its route was muted; muting it again",
            d.name, HEADSET_INPUT_MUTE_MSEC))
        setRoute (d, now.index, true, nil, true)
      end
      return false
    end)
  end
end

-- Bring the input to the intended mute where the tools look. Never unmutes on
-- the way: while muted, the loopback stays muted until the real headset input
-- shows the mute. During a switch only that protection runs: keep-mute writes
-- the route then.
reconcile = function (d)
  if not d.node then
    return
  end
  protect (d)
  if d.transition then
    return
  end
  local route = d.route
  if route and route.mute ~= nil then
    if route.mute ~= d.intent and mayWrite (d, "route") then
      setRoute (d, route.index, d.intent, nil, true)
    end
    -- the loopback's mute goes only once the Route holds the intended mute
    -- and, while muted, the real headset input shows it
    if d.props_mute == true and route.mute == d.intent and mayWrite (d, "props")
        and (not d.intent or not d.real or readPropsMute (d.real) == true) then
      setPropsMute (d, false)
    end
  elseif d.props_mute ~= nil and d.props_mute ~= d.intent
      and mayWrite (d, "props") then
    setPropsMute (d, d.intent)
  end
end

-- Take in a new reading of the device-0 Route from bluez5: what the script
-- knows, and the intent when someone else changed the mute. Writes nothing;
-- returns whether the input needs reconciling, which callers do once, after
-- taking in everything.
local function observeRoute (d, route)
  local was = d.route
  if was == route
      or (was and route and was.mute == route.mute and was.index == route.index) then
    return false
  end
  -- any reading taken in counts: a read answer older than it is not used
  d.observed = (d.observed or 0) + 1
  d.route = route
  log:debug (string.format ("%s: input route %s -> %s (intent %s, %s)",
      d.name, was and tostring (was.mute) or "none",
      route and tostring (route.mute) or "none", tostring (d.intent),
      d.transition and "switching" or "steady"))

  if not route then
    log:info (d.name .. ": input route gone; the input's own mute takes over")
    d.set_without_route = false
    endTransition (d)
    return true
  end
  if not was then
    log:info (d.name .. ": input route appeared")
    -- a mute it carries may be the user's (bluez5 keeps a node's mute while
    -- its route is away), unless you set the mute since the route went
    if route.mute and not d.intent and not d.set_without_route then
      setIntent (d, true, "found on the input route as it appeared")
    end
    d.set_without_route = false
    beginTransition (d)
    return true
  end
  if route.mute == nil or route.mute == was.mute then
    return false
  end
  if consume (d, "route", route.mute) then
    return true
  end
  -- an established route's mute is written by the user, or by this script
  -- (pending above): WirePlumber's restores of it go through keep-mute
  d.retries ["route"] = 0
  setIntent (d, route.mute, "set on the device route")
  return true
end

-- A new reading of the device-0 Route from bluez5, on its own.
local function onRoute (d, route)
  if observeRoute (d, route) then
    reconcile (d)
  end
end

local function onPropsChanged (d)
  local mute = readPropsMute (d.node)
  if mute == nil or mute == d.props_mute then
    return
  end
  d.props_mute = mute
  d.observed = (d.observed or 0) + 1
  log:debug (string.format ("%s: input's own mute -> %s (intent %s)",
      d.name, tostring (mute), tostring (d.intent)))
  -- where this mute belongs depends on whether a Route exists now: take that
  -- in first, then this change, and reconcile once
  local route, present = readRoute (d.name)
  if present then
    observeRoute (d, route)
  end
  if consume (d, "props", mute) then
    -- a read for a done restore held back by this write can be asked now
    if d.retained ~= nil and not d.retire_reading then
      askRetired (d)
    end
    reconcile (d)
    return
  end
  d.retries ["props"] = 0
  if route and route.mute ~= nil then
    if mute then
      -- the watcher: a mute nobody would see; make it the one everybody sees
      log:warning (string.format (
          "%s: moved a mute set on the input itself to the device mute (profile %s)",
          d.name, d.profile or "?"))
      setIntent (d, true, "set on the input itself while the device route exists")
    end
  else
    d.set_without_route = true
    setIntent (d, mute, "set on the input itself")
  end
  reconcile (d)
end

-- The SpaDevice's params changed (in bluez5's order, on the main thread);
-- spa is the emitter, used only within this call.
local function onSpaParams (name, spa, id)
  local d = devices [name]
  if not d then
    return
  end
  if id == "Route" then
    onRoute (d, findRoute (spa:iterate_params ("Route")))
  elseif id == "Profile" then
    local profile = readProfile (spa)
    if profile ~= d.profile then
      d.profile = profile
      log:debug (d.name .. ": profile " .. tostring (profile))
      beginTransition (d)
      reconcile (d)
    end
  end
end

-- The real headset input, whose mute the Route sets. bluez5 creates it
-- unmuted, and WirePlumber restores the input route on a profile switch only
-- if that route was not already active, so a new headset input can come up
-- live under a muted route: mute it right away (the loopback is muted
-- meanwhile by protect).
local function watchHeadsetInput (d, node)
  local node_serial = serial (node)
  if d.real_serial == node_serial then
    return
  end
  d.real = node
  d.real_serial = node_serial
  d.real_fixes = 0
  local name = d.name
  node:connect ("params-changed", function (_, id)
    local current = devices [name]
    if id == "Props" and current and current.real_serial == node_serial then
      reconcile (current)
    end
  end)
  local now = readRoute (name)
  if d.intent and now and now.mute and readPropsMute (node) == false then
    log:warning (string.format (
        "%s: the new headset input came up live under a muted route; muting it", name))
    setRoute (d, now.index, true, nil, true)
  end
  reconcile (d)
end

local function deviceName (event, node)
  local device = event:get_source ():call ("get-object-manager", "device"):lookup {
    Constraint { "bound-id", "=", node.properties ["device.id"], type = "gobject" },
  }
  if device and device.properties ["device.api"] == "bluez5" then
    return device.properties ["device.name"], device
  end
  return nil
end

local function newRecord (name)
  local d = {
    name = name,
    pending = {},
    retries = {},
    generation = 0,
  }
  d.intent = rememberedMute (name)
  if state_table [name] == nil then
    state_table [name] = tostring (d.intent)
    saveState ()
  end
  local spa = spaDevice (name)
  d.route = spa and findRoute (spa:iterate_params ("Route"))
  d.profile = spa and readProfile (spa)
  devices [name] = d
  return d
end

local function track (name, device, node)
  local d = devices [name] or newRecord (name)
  if d.node_serial == serial (node) then
    return
  end
  local replacement = d.input_name ~= nil
  d.node = node
  d.node_serial = serial (node)
  d.input_name = node.properties ["node.name"]
  d.props_mute = readPropsMute (node)
  if replacement then
    -- a new input after an earlier one went: what was pending or seen for that
    -- one does not apply; start from the device as it is now
    for _, source in pairs (d.pending) do
      source:destroy ()
    end
    d.pending, d.retries = {}, {}
    release (d)
    d.route = readRoute (name)
    local spa = spaDevice (name)
    d.profile = spa and readProfile (spa)
  end
  -- it starts from what talkd owes it, else from the remembered mute (as on a
  -- new connection), never from a flip kept in memory
  local owed = talkd_owed [d.input_name]
  if owed ~= nil then
    d.intent = owed
  elseif replacement then
    d.intent = rememberedMute (name)
  end
  remember (d)
  local node_serial = d.node_serial
  node:connect ("params-changed", function (_, id)
    local current = devices [name]
    if id == "Props" and current and current.node_serial == node_serial then
      onPropsChanged (current)
    end
  end)
  -- the headset input may exist already (WirePlumber restarted in headset mode)
  local real = cutils.get_object_manager ("node"):lookup {
    Constraint { "media.class", "matches", "Audio/Source/Internal" },
    Constraint { "device.id", "=", tostring (device ["bound-id"]) },
    Constraint { "card.profile.device", "=", tostring (INPUT_DEVICE) },
  }
  if real then
    watchHeadsetInput (d, real)
  end
  log:info (node, string.format ("%s: keeping its input's mute; remembered mute %s",
      name, tostring (d.intent)))
  reconcile (d)
end

-- Drops everything tied to the device's current lifetime; the remembered mute
-- stays in the state file.
local function untrack (name)
  local d = devices [name]
  if not d then
    return
  end
  endTransition (d)
  release (d)
  if d.checking_real then
    d.checking_real:destroy ()
  end
  for _, source in pairs (d.pending) do
    source:destroy ()
  end
  d.node, d.real = nil, nil
  devices [name] = nil
  saveState ()
end

-- The monitor created a bluez device (this hook runs once create-device.lua
-- has activated it): remember where to find its SpaDevice, under the name
-- name-device.lua gave it, and watch its params from bluez5 directly. A new
-- SpaDevice under a known name (the monitor was replaced, say) starts a new
-- record.
SimpleEventHook {
  name = "bluez-loopback-mute/note-device",
  after = { "monitor/bluez/name-device", "monitor/bluez/create-device" },
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "create-bluez-device" },
    },
  },
  execute = function (event)
    local props = event:get_data ("device-properties")
    local name = props and props ["device.name"]
    local id = event:get_data ("device-sub-id")
    if not name or id == nil then
      return
    end
    monitor = event:get_subject ()
    spa_ids [name] = id
    local spa = spaDevice (name)
    if not spa then
      return
    end
    untrack (name)
    local generation = (generations [name] or 0) + 1
    generations [name] = generation
    spa:connect ("params-changed", function (emitter, param)
      if generations [name] == generation then
        onSpaParams (name, emitter, param)
      end
    end)
    log:debug (string.format ("%s: watching the device's params (sub-id %s)",
        name, tostring (id)))
  end
}:register ()

SimpleEventHook {
  name = "bluez-loopback-mute/forget-device",
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "remove-bluez-device" },
    },
  },
  execute = function (event)
    local id = event:get_data ("device-sub-id")
    for name, spa_id in pairs (spa_ids) do
      if spa_id == id then
        spa_ids [name] = nil
        generations [name] = (generations [name] or 0) + 1
        untrack (name)
      end
    end
  end
}:register ()

SimpleEventHook {
  name = "bluez-loopback-mute/track",
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "node-added" },
      Constraint { "media.class", "matches", "Audio/Source" },
      Constraint { "device.id", "+" },
      Constraint { "bluez5.loopback", "=", "true", type = "pw" },
    },
  },
  execute = function (event)
    local node = event:get_subject ()
    local name, device = deviceName (event, node)
    if name then
      track (name, device, node)
    end
  end
}:register ()

SimpleEventHook {
  name = "bluez-loopback-mute/track-headset-input",
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "node-added" },
      Constraint { "media.class", "matches", "Audio/Source/Internal" },
      Constraint { "device.id", "+" },
      Constraint { "card.profile.device", "=", tostring (INPUT_DEVICE) },
    },
  },
  execute = function (event)
    local node = event:get_subject ()
    local name = deviceName (event, node)
    local d = name and devices [name]
    if d then
      watchHeadsetInput (d, node)
    end
  end
}:register ()

SimpleEventHook {
  name = "bluez-loopback-mute/forget-nodes",
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "node-removed" },
      Constraint { "media.class", "matches", "Audio/Source*" },
      Constraint { "device.id", "+" },
    },
  },
  execute = function (event)
    local node_serial = event:get_properties () ["object.serial"]
    for _, d in pairs (devices) do
      if d.node_serial == node_serial then
        d.node = nil
        d.node_serial = nil
        d.props_mute = nil
      end
      if d.real_serial == node_serial then
        d.real = nil
        d.real_serial = nil
      end
    end
  end
}:register ()

-- WirePlumber is about to apply the routes it restored. Set the input route
-- here instead, on bluez5 directly: first take in what the device has now (a
-- change WirePlumber has not seen yet), then write the mute you last set with
-- the rest of the restored props.
SimpleEventHook {
  name = "bluez-loopback-mute/keep-mute",
  after = { "device/apply-route-props" },
  before = { "device/apply-routes" },
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "select-routes" },
    },
  },
  execute = function (event)
    local device = event:get_subject ()
    local d = devices [device.properties ["device.name"]]
    local selected = d and event:get_data ("selected-routes")
    if not selected then
      return
    end
    local input = nil
    for device_id, _ in pairs (selected) do
      if tonumber (device_id) == INPUT_DEVICE then
        input = device_id
      end
    end
    local now = input and readRoute (d.name)
    if now then
      observeRoute (d, now)
    end
    -- taken after that: a route appearing just now belongs to this restore
    event:set_data ("bluez-loopback-mute.generation", d.generation)
    local routes = {}
    for device_id, value in pairs (selected) do
      if device_id ~= input or not now then
        routes [device_id] = value
      else
        local route = Json.Raw (value):parse ()
        local props = route.props or {}
        if props.mute ~= d.intent then
          log:info (string.format (
              "%s: restoring the input route with mute %s, not the stored %s",
              d.name, tostring (d.intent), tostring (props.mute)))
        end
        props.mute = nil
        for key, kind in pairs ({ channelVolumes = "Spa:Float",
            channelMap = "Spa:Enum:AudioChannel",
            iec958Codecs = "Spa:Enum:AudioIEC958Codec" }) do
          if props [key] then
            table.insert (props [key], 1, kind)
            props [key] = Pod.Array (props [key])
          end
        end
        -- as apply-routes.lua does for a route with no stored volume
        if not props.channelVolumes then
          props.channelVolumes = Pod.Array { "Spa:Float",
            tonumber (device.properties ["device.routes.default-source-volume"])
                or Settings.get_float ("device.routes.default-source-volume") }
        end
        setRoute (d, route.index, d.intent, props, false)
      end
    end
    event:set_data ("selected-routes", routes)
  end
}:register ()

-- The restore is applied: unless another switch began meanwhile, the input
-- route is the device's own again; finish any pending handoff.
SimpleEventHook {
  name = "bluez-loopback-mute/after-restore",
  after = { "device/apply-routes" },
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "select-routes" },
    },
  },
  execute = function (event)
    local d = devices [event:get_subject ().properties ["device.name"]]
    if not d then
      return
    end
    -- no other switch began since keep-mute: this restore is the latest, and
    -- a route the read below finds appearing belongs to it
    local latest = event:get_data ("bluez-loopback-mute.generation") == d.generation
    local route, present = readRoute (d.name)
    if present then
      observeRoute (d, route)
    end
    if latest then
      endTransition (d)
    end
    reconcile (d)
  end
}:register ()

-- Take bluez5's current reading of the input route as what you set.
local function adoptRoute (d, route)
  d.route = route
  d.set_without_route = false
  if route.mute ~= d.intent then
    setIntent (d, route.mute, "the device route once talkd's restore was done")
    reconcile (d)
  else
    remember (d)
  end
end

-- PipeWire's answer to askRetired: the loopback's mute (nil: no answer).
local function resolveRetired (d, token, node_serial, asked, answered)
  if devices [d.name] ~= d or d.retire_token ~= token then
    return
  end
  d.retire_reading = false
  if answered == nil or d.node_serial ~= node_serial then
    return -- asked again at the input's next change
  end
  local route, present = readRoute (d.name)
  if present and route and route.mute ~= nil then
    release (d)
    adoptRoute (d, route)
    return
  end
  -- the answer can predate a reading this script took in meanwhile (a Route
  -- that came and went, through bluez5 directly) and this script's own write
  -- to the loopback: ask again, once that has landed
  if d.observed ~= asked.observed or d.generation ~= asked.generation then
    log:debug (d.name .. ": the loopback's mute was read before a change seen since; asking again")
    askRetired (d)
    return
  end
  if pendingOn (d, "props") then
    return -- asked again when the write shows up
  end
  release (d)
  state_table [d.name] = tostring (answered)
  saveState ()
end

-- Ask PipeWire for the loopback's own mute, for a done restore. With nothing
-- taken in meanwhile and no write of this script's own under way, the answer
-- is at least as new as talkd's write and as all this script saw before it,
-- and later changes reach this script after it, so it is what to remember (the
-- Route instead, if one exists by then). The live mute is left to this
-- script's own observations.
askRetired = function (d)
  d.retire_reading = true
  local token, node_serial = d.retire_token, d.node_serial
  local asked = { observed = d.observed, generation = d.generation }
  d.node:enum_params ("Props", function (it, e)
    local answered = nil
    if not e then
      for p in it:iterate () do
        local props = cutils.parseParam (p, "Props")
        if props and props.mute ~= nil then
          answered = props.mute
          break
        end
      end
    end
    resolveRetired (d, token, node_serial, asked, answered)
  end)
end

-- talkd's restore for this input is done: it put back `restored`. Where a Route
-- exists, bluez5's own reading of it is current: take it as what you set.
-- Without one, this script's view of the loopback's own mute can lag talkd's
-- write: keep remembering `restored` until PipeWire's answer for the loopback
-- (askRetired) says what to remember. With the input gone first, `restored`
-- stays remembered.
local function retire (d, restored)
  release (d)
  local route, present = readRoute (d.name)
  if present and route and route.mute ~= nil then
    adoptRoute (d, route)
    return
  end
  d.retained = restored
  remember (d)
  askRetired (d)
end

-- What talkd owes changed. An entry that appears (or changes) is the mute to
-- remember; one that goes means talkd's restore is done (retire). With the
-- input gone, the remembered mute stays. A first snapshot (a new metadata
-- object) completes nothing; a value not understood changes nothing.
local function onTalkdOwed (value, first)
  local owed = {}
  if value then
    local json = Json.Raw (value)
    local parsed = json:is_object () and json:parse () or nil
    for _, mute in pairs (parsed or {}) do
      if type (mute) ~= "boolean" then
        parsed = nil
      end
    end
    if not parsed then
      log:warning ("ignoring what talkd owes, not understood: " .. value)
      return
    end
    for name, mute in pairs (parsed) do
      owed [name] = mute
    end
  end
  log:debug ("talkd owes: " .. tostring (value))
  local was = talkd_owed
  talkd_owed = owed
  for _, d in pairs (devices) do
    local name = d.input_name
    if name and was [name] ~= owed [name] then
      if owed [name] ~= nil then
        release (d)
        remember (d)
      elseif d.node and not first then
        retire (d, was [name])
      end
    end
  end
end

-- a global: nothing else keeps an object manager alive
default_metadata_om = ObjectManager {
  Interest {
    type = "metadata",
    Constraint { "metadata.name", "=", "default" },
  },
}
default_metadata_om:connect ("object-added", function (_, metadata)
  onTalkdOwed (metadata:find (0, "talkd.owed"), true)
  metadata:connect ("changed", function (m, subject, key)
    -- a whole subject cleared comes without a key
    if subject == 0 and (key == "talkd.owed" or key == nil) then
      onTalkdOwed ((m:find (0, "talkd.owed")))
    end
  end)
end)
default_metadata_om:activate ()
