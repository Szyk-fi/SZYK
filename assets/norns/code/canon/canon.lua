-- canon
-- a Portamax norns script
--
-- a short tune is invented, then
-- sung as a round: three voices
-- enter one after another, each
-- with its own place in the room.
-- the low voice may sing it
-- upside down.
--
-- E2 entry gap   E3 brightness
-- K2 new tune   K3 mirror voice 3
-- pads: new tune in that key
-- (params: scale, root, length,
--  rests, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local tune = {}
local s = 0
local VOICES = {
  { pan = -0.7, octave = 7, pw = 0.5, amp = 0.2 },
  { pan = 0.7, octave = 7, pw = 0.3, amp = 0.17 },
  { pan = 0.0, octave = 0, pw = 0.6, amp = 0.22 },
}
local lit = { 0, 0, 0 }
local mirror = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function new_tune()
  tune = {}
  local len = params:get("length")
  local deg = 1
  for i = 1, len do
    if i == 1 or i == len - 1 then
      tune[i] = 1
      deg = 1
    elseif math.random() < params:get("rests") / 100 and i % 2 == 0 then
      tune[i] = false
    else
      -- mostly steps, now and then a leap of a third or fourth
      local moves = { -1, 1, -1, 1, 2, -2, 3, -3 }
      deg = util.clamp(deg + moves[math.random(1, #moves)], -2, 6)
      tune[i] = deg
    end
  end
  tune[len] = false
  s = 0
end

local function play_step()
  local len = #tune
  local gap = params:get("gap")
  for v, voice in ipairs(VOICES) do
    local k = s - (v - 1) * gap
    if k >= 0 then
      local d = tune[(k % len) + 1]
      if d then
        if v == 3 and mirror then d = 2 - d end
        local idx = util.clamp(d + 3 + voice.octave, 1, #scale)
        engine.pan(voice.pan)
        engine.pw(voice.pw)
        engine.amp(voice.amp)
        engine.hz(MusicUtil.note_num_to_freq(scale[idx]))
        lit[v] = 15
      end
    end
  end
  s = s + 1
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CANON")
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 43, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("length", "length", 8, 16, 16)
  params:add_number("gap", "entry gap", 1, 8, 4)
  params:add_number("rests", "rests %", 0, 60, 25)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.8, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_tune()
  play_step()

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local r = msg.note
      while r > 60 do r = r - 12 end
      while r < 36 do r = r + 12 end
      params:set("root", r)
      new_tune()
      play_step()
    end
  end

  clock.run(function()
    while true do
      clock.sync(1 / 2)
      play_step()
    end
  end)
  local mt = metro.init(function()
    for v = 1, 3 do lit[v] = math.max(0, lit[v] - 1) end
    redraw()
  end, 1 / 20)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("gap", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_tune()
  elseif n == 3 then mirror = not mirror end
end

function redraw()
  screen.clear()
  local len = #tune
  local cw = 112 / len
  local gap = params:get("gap")
  for v = 1, 3 do
    local top = 1 + (v - 1) * 18
    screen.level(math.max(2, lit[v]))
    screen.move(0, top + 10)
    screen.text(v)
    for i = 1, len do
      local d = tune[i]
      local k = s - 1 - (v - 1) * gap
      local here = k >= 0 and (k % len) + 1 == i
      if d then
        if v == 3 and mirror then d = 2 - d end
        local y = top + 14 - (d + 2) * 1.6
        screen.level(here and 15 or 4)
        screen.rect(10 + (i - 1) * cw, y, math.max(2, cw - 2), 2)
        screen.fill()
      end
      if here then
        screen.level(6)
        screen.move(10 + (i - 1) * cw, top + 17)
        screen.line(10 + i * cw - 2, top + 17)
        screen.stroke()
      end
    end
  end
  screen.level(15)
  screen.move(127, 63)
  screen.text_right(mirror and "canon (mirror)" or "canon")
  screen.update()
end
