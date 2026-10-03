-- lander
-- a Portamax norns script
--
-- a lunar lander falls toward a
-- pad, steered by an autopilot.
-- the altimeter beeps lower as
-- it drops, every burn sings,
-- and touchdown is a chord:
-- bright if soft, dark if hard.
--
-- E2 gravity     E3 pilot nerve
-- K2 new descent K3 burn (manual)
-- pads: burn, pitched
-- (params: key, fuel)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local terrain = {}
local pad_x = 64
local L = {}
local state = "fly"
local wait = 0
local burning = false
local manual = 0
local beep_t = 0
local burn_note_t = 0
local result = ""
local scale = {}
local landings = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("key"), "Lydian", 15)
end

local function make_terrain()
  terrain = {}
  local h = 54
  pad_x = math.random(24, 104)
  for x = 0, 128, 4 do
    if math.abs(x - pad_x) <= 10 then
      terrain[x // 4] = 56
    else
      h = util.clamp(h + math.random(-4, 4), 44, 62)
      terrain[x // 4] = h
    end
  end
end

local function ground(x)
  local i = util.clamp(math.floor(x / 4), 0, 32)
  return terrain[i] or 60
end

local function tone(n, amp, rel, cut, pan)
  engine.pan(pan or 0)
  engine.pw(0.5)
  engine.cutoff(cut or 2000)
  engine.release(rel or 0.3)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function new_descent()
  make_terrain()
  L = { x = math.random(10, 118), y = 4, vx = (math.random() - 0.5) * 8, vy = 2, fuel = params:get("fuel") }
  state = "fly"
  result = ""
  -- radio chirp: we're on our way down
  tone(scale[13], 0.15, 0.15, 5000, 0)
  tone(scale[15], 0.12, 0.2, 5000, 0)
end

local function altitude()
  return ground(L.x) - (L.y + 5)
end

local function land()
  local v = L.vy
  local on_pad = math.abs(L.x - pad_x) <= 9
  state = "down"
  wait = 3
  local root = params:get("key") - 12
  local chord
  if v < 9 and on_pad then
    chord = { 0, 7, 11, 16, 19, 26 } -- maj7(9): a soft landing on the pad
    result = "the eagle has landed"
    landings = landings + 1
  elseif v < 9 then
    chord = { 0, 7, 14, 17 } -- sus: safe, but off the pad
    result = "down, off the pad"
  else
    chord = { 0, 3, 6, 10, 13 } -- half-diminished: a hard landing
    result = "crunch"
  end
  for i, iv in ipairs(chord) do
    tone(root + iv, 0.16, 2.8, 2400, util.linlin(1, #chord, -0.6, 0.6, i))
  end
end

local function step(dt)
  if state == "down" then
    wait = wait - dt
    if wait <= 0 then new_descent() end
    return
  end
  local g = params:get("gravity")
  local thrust = g * 2.6
  local alt = altitude()
  -- autopilot: burn when stopping distance at full thrust (with a
  -- margin set by the pilot's nerve) reaches the ground; aim for 4 px/s
  local decel = thrust - g
  local stop_dist = (L.vy * L.vy - 16) / (2 * decel)
  local want = (L.vy > 4) and (stop_dist >= alt * params:get("nerve")) or (L.vy > 7)
  -- steer over the pad with small sideways nudges
  local side = util.clamp((pad_x - L.x) * 0.15 - L.vx * 0.5, -1, 1)
  burning = (want or manual > 0) and L.fuel > 0
  manual = math.max(0, manual - dt)
  if burning then
    L.vy = L.vy - thrust * dt
    L.fuel = L.fuel - dt
  end
  if L.fuel > 0 then
    L.vx = L.vx + side * 6 * dt
    L.fuel = L.fuel - math.abs(side) * dt * 0.2
  end
  L.vy = L.vy + g * dt
  L.x = util.clamp(L.x + L.vx * dt, 3, 124)
  L.y = L.y + L.vy * dt
  -- altimeter: beeps faster and lower as the ground comes up
  local a = math.max(0, altitude())
  beep_t = beep_t - dt
  if beep_t <= 0 then
    local deg = util.clamp(math.floor(util.linlin(0, 50, 1, 15, a)), 1, 15)
    tone(scale[deg] + 12, 0.1, 0.08, 4000, util.linlin(0, 127, -0.5, 0.5, L.x))
    beep_t = util.linlin(0, 50, 0.12, 0.45, a)
  end
  -- burns sing a fifth below the altimeter, re-struck while they last
  burn_note_t = burn_note_t - dt
  if burning and burn_note_t <= 0 then
    local deg = util.clamp(math.floor(util.linlin(0, 50, 1, 15, a)), 1, 15)
    engine.pan(0)
    engine.pw(0.2)
    engine.cutoff(900)
    engine.release(0.35)
    engine.amp(0.28)
    engine.hz(MusicUtil.note_num_to_freq(scale[deg] - 7))
    burn_note_t = 0.16
  end
  if not burning then burn_note_t = 0 end
  if altitude() <= 0 then
    L.y = ground(L.x) - 5
    land()
  end
end

function init()
  params:add_separator("LANDER")
  params:add_number("key", "key", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_scale)
  params:add_control("gravity", "gravity", controlspec.new(1, 20, 'lin', 0, 6, 'px/s2'))
  params:add_control("nerve", "pilot nerve", controlspec.new(0.6, 1.4, 'lin', 0, 0.9, ''))
  params:add_control("fuel", "fuel", controlspec.new(3, 30, 'lin', 0, 14, 's'))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_descent()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      manual = 0.3
      tone(msg.note, 0.2, 0.4, 1500, 0)
    end
  end
  local last = util.time()
  local frame = metro.init(function()
    local now = util.time()
    step(math.min(0.1, now - last))
    last = now
    redraw()
  end, 1 / 40)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("gravity", d)
  elseif n == 3 then params:delta("nerve", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_descent()
  elseif n == 3 then manual = 0.4 end
  redraw()
end

function redraw()
  screen.clear()
  -- stars
  screen.level(2)
  for i = 1, 12 do
    screen.pixel((i * 37) % 128, (i * 13) % 30)
    screen.fill()
  end
  -- ground
  screen.level(6)
  screen.move(0, terrain[0])
  for i = 1, 32 do screen.line(i * 4, terrain[i]) end
  screen.stroke()
  -- the pad
  screen.level(15)
  screen.move(pad_x - 9, 56)
  screen.line(pad_x + 9, 56)
  screen.stroke()
  -- the lander
  local x, y = math.floor(L.x), math.floor(L.y)
  screen.level(state == "down" and result == "crunch" and 5 or 13)
  screen.rect(x - 2, y - 3, 5, 4)
  screen.stroke()
  screen.move(x - 2, y + 1)
  screen.line(x - 4, y + 5)
  screen.stroke()
  screen.move(x + 3, y + 1)
  screen.line(x + 5, y + 5)
  screen.stroke()
  if burning and state == "fly" then
    screen.level(math.random(8, 15))
    screen.move(x - 1, y + 2)
    screen.line(x + 0.5, y + 4 + math.random(2, 5))
    screen.line(x + 2, y + 2)
    screen.stroke()
  end
  -- instruments
  screen.level(5)
  screen.move(0, 6)
  screen.text("alt " .. math.floor(math.max(0, altitude())))
  screen.move(44, 6)
  screen.text("v " .. string.format("%.1f", L.vy))
  screen.move(127, 6)
  screen.text_right("fuel " .. math.floor(math.max(0, L.fuel)))
  if state == "down" then
    screen.level(15)
    screen.move(64, 24)
    screen.text_center(result)
  end
  screen.level(3)
  screen.move(127, 14)
  screen.text_right(landings)
  screen.update()
end
