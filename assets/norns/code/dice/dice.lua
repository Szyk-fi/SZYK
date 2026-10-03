-- dice
-- a Portamax norns script
--
-- three dice are thrown every few
-- bars. the first picks the chord
-- (I to vi), the second how many
-- hits the bar gets, the third the
-- voicing: odd climbs, even falls,
-- five or six adds the seventh.
--
-- E2 bars per throw   E3 brightness
-- K2 throw now   K3 hold the dice
-- pads: change key
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PIPS = {
  { { 2, 2 } }, { { 1, 1 }, { 3, 3 } }, { { 1, 1 }, { 2, 2 }, { 3, 3 } },
  { { 1, 1 }, { 3, 1 }, { 1, 3 }, { 3, 3 } }, { { 1, 1 }, { 3, 1 }, { 2, 2 }, { 1, 3 }, { 3, 3 } },
  { { 1, 1 }, { 3, 1 }, { 1, 2 }, { 3, 2 }, { 1, 3 }, { 3, 3 } },
}
local NUMERALS = { "I", "ii", "iii", "IV", "V", "vi" }
local dice = { 1, 4, 3 }
local shown = { 1, 4, 3 }
local tumbling = 0
local held = false
local stepn = 0
local bar = 0
local scale = {}
local chord = {}
local lit = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function build_chord()
  -- stack thirds on the die's scale degree
  local d = dice[1]
  chord = { scale[d], scale[d + 2], scale[d + 4] }
  if dice[3] >= 5 then chord[4] = scale[d + 6] end
  if dice[3] % 2 == 0 then
    local r = {}
    for i = #chord, 1, -1 do r[#r + 1] = chord[i] + 12 end
    chord = r
  end
end

local function throw()
  tumbling = 4
end

local function hit(i)
  return ((i * dice[2]) % 8) < dice[2]
end

local function tick()
  stepn = stepn % 8 + 1
  if tumbling > 0 then
    tumbling = tumbling - 1
    for i = 1, 3 do shown[i] = math.random(6) end
    engine.amp(0.07)
    engine.release(0.15)
    engine.hz(MusicUtil.note_num_to_freq(scale[math.random(8, 14)] + 12))
    if tumbling == 0 then
      for i = 1, 3 do dice[i] = math.random(6) end
      shown = { dice[1], dice[2], dice[3] }
      build_chord()
    end
  end
  if stepn == 1 then
    bar = bar + 1
    engine.amp(0.3)
    engine.release(2.4)
    engine.pan(0)
    engine.hz(MusicUtil.note_num_to_freq(scale[dice[1]] - 12))
    if not held and bar % params:get("bars") == 0 then throw() end
  end
  if hit(stepn - 1) then
    lit = (lit % #chord) + 1
    engine.amp(0.22)
    engine.release(0.8)
    engine.pan((lit - 2.5) / 3)
    engine.hz(MusicUtil.note_num_to_freq(chord[lit]))
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("DICE")
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", function() build_scale() build_chord() end)
  params:add_number("root", "root", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", function() build_scale() build_chord() end)
  params:add_number("bars", "bars per throw", 1, 8, 2)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2000, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  build_chord()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", msg.note) end
  end
  clock.run(function()
    while true do
      tick()
      redraw()
      clock.sync(1 / 2)
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("bars", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then throw() elseif n == 3 then held = not held end
end

local function draw_die(x, y, face, level)
  screen.level(level)
  screen.rect(x, y, 22, 22)
  screen.stroke()
  for _, p in ipairs(PIPS[face]) do
    screen.circle(x + p[1] * 6 - 1, y + p[2] * 6 - 1, 1.5)
    screen.fill()
  end
end

function redraw()
  screen.clear()
  for i = 1, 3 do
    local wob = tumbling > 0 and math.random(-2, 2) or 0
    draw_die(16 + (i - 1) * 36 + wob, 16 + wob, shown[i], tumbling > 0 and 6 or 15)
  end
  for s = 1, 8 do
    screen.level(s == stepn and 15 or (hit(s - 1) and 6 or 1))
    screen.rect(16 + (s - 1) * 12, 45, 8, 3)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(held and "dice (held)" or "dice")
  screen.move(127, 8)
  screen.text_right(NUMERALS[dice[1]] .. (dice[3] >= 5 and "7" or ""))
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(params:get("root"), false) .. " " .. dice[2] .. " hits")
  screen.move(127, 62)
  screen.text_right("every " .. params:get("bars"))
  screen.update()
end
