-- harp
-- a Portamax norns script
--
-- a harp of twenty-two strings
-- tuned to a scale. glissandi
-- sweep from wherever the hand
-- rests to a new target. a pad
-- sets the target (pad 1 low,
-- pad 8 high); left alone it
-- wanders by itself.
--
-- E2 scale   E3 sweep speed
-- K2 new target   K3 flourish
-- (params: root, ring)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local NSTR = 22
local SCALES = { "Major Pentatonic", "Lydian", "Dorian", "Harmonic Minor", "Whole Tone" }
local strings = {}
local hand = 8
local target = 15
local glow = {}
local flourish = false

local function build()
  strings = MusicUtil.generate_scale_of_length(params:get("root"), SCALES[params:get("scale")], NSTR)
end

local function pluck(i, amp)
  engine.pan((i / NSTR - 0.5) * 1.4)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(strings[i]))
  glow[i] = 15
end

local function sweep()
  local dirn = target > hand and 1 or -1
  local n = math.abs(target - hand)
  for k = 1, n do
    hand = hand + dirn
    -- a gliss swells in the middle and thins at the ends
    local shape = math.sin(k / (n + 1) * math.pi)
    pluck(hand, 0.12 + 0.14 * shape)
    clock.sleep(params:get("speed") * (1.2 - 0.4 * shape))
  end
  pluck(hand, 0.3)
end

function init()
  params:add_separator("HARP")
  params:add_option("scale", "scale", SCALES, 1)
  params:set_action("scale", build)
  params:add_number("root", "root", 36, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_control("speed", "sweep speed", controlspec.new(0.02, 0.15, 'exp', 0, 0.05, 's'))
  params:add_control("ring", "ring", controlspec.new(0.5, 4, 'lin', 0, 2.2, 's'))
  params:set_action("ring", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2600)
  engine.pw(0.45)
  for i = 1, NSTR do glow[i] = 0 end
  build()
  math.randomseed(os.time())
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      target = util.clamp(math.floor(util.linlin(60, 72, 2, NSTR, msg.note)), 1, NSTR)
    end
  end
  clock.run(function()
    while true do
      sweep()
      if flourish then
        target = hand > NSTR / 2 and 1 or NSTR
        sweep()
      end
      clock.sync(1)
      -- drift to somewhere new, at least a few strings away
      local t = math.random(1, NSTR)
      if math.abs(t - hand) < 5 then t = (t + 9 - 1) % NSTR + 1 end
      target = t
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      for i = 1, NSTR do glow[i] = math.max(0, glow[i] - 1) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("scale", d)
  elseif n == 3 then params:delta("speed", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then target = math.random(1, NSTR)
  elseif n == 3 then flourish = not flourish end
end

function redraw()
  screen.clear()
  for i = 1, NSTR do
    local x = 8 + (i - 1) * 5.3
    local top = 14 + (i - 1) * 0.9
    screen.level(math.max(2, glow[i]))
    screen.move(x, top)
    screen.line(x, 52)
    screen.stroke()
  end
  screen.level(15)
  screen.rect(6 + (hand - 1) * 5.3, 53, 5, 2)
  screen.fill()
  screen.level(6)
  screen.rect(7 + (target - 1) * 5.3, 11, 3, 2)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(flourish and "harp *" or "harp")
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.lower(SCALES[params:get("scale")]))
  screen.move(127, 62)
  screen.text_right(params:string("speed"))
  screen.update()
end
