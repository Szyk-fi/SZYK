-- fibonacci
-- a Portamax norns script
--
-- each number is the sum of the two
-- before it. taken modulo the length
-- of a scale the sequence loops (its
-- pisano period), giving a melody
-- that always comes home. a golden
-- spiral turns as it plays.
--
-- E2 modulus   E3 scale
-- K2 new seeds   K3 pause
-- pads: root note
-- (params: step, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PHI = (1 + math.sqrt(5)) / 2
local DIVS = { 1 / 4, 1 / 2, 1 }
local a, b = 0, 1
local seed = { 0, 1 }
local count = 0
local period = 0
local scale = {}
local root = 48
local paused = false
local last = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), params:get("mod"))
end

local function pisano(m)
  -- how many steps until the seed pair comes back
  local x, y = seed[1] % m, seed[2] % m
  for i = 1, 6 * m * m do
    x, y = y, (x + y) % m
    if x == seed[1] % m and y == seed[2] % m then return i end
  end
  return 0
end

local function reset()
  local m = params:get("mod")
  a, b = seed[1] % m, seed[2] % m
  count = 0
  period = pisano(m)
  build_scale()
end

local function step()
  local m = params:get("mod")
  last = a
  local note = scale[util.clamp(a + 1, 1, #scale)]
  engine.amp(a == 0 and 0.3 or 0.2)
  engine.pan(util.linlin(0, m - 1, -0.5, 0.5, a))
  engine.hz(MusicUtil.note_num_to_freq(note))
  -- every return to zero rings the root an octave down
  if a == 0 then engine.hz(MusicUtil.note_num_to_freq(root - 12)) end
  a, b = b, (a + b) % m
  count = count + 1
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("FIBONACCI")
  params:add_number("mod", "modulus", 3, 16, 10)
  params:set_action("mod", reset)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_option("div", "step", { "1/16", "1/8", "1/4" }, 2)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 1.1, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2000)
  engine.pw(0.5)
  reset()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 12 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then step() end
      redraw()
      clock.sync(DIVS[params:get("div")])
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("mod", d)
  elseif n == 3 then params:delta("scale", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    seed = { math.random(0, 9), math.random(1, 9) }
    reset()
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  -- golden spiral: radius grows by phi every quarter turn
  local cx, cy = 84, 34
  local rot = count * 0.12
  local function at(t)
    local r = 0.3 * PHI ^ (t / (math.pi / 2))
    return cx + r * math.cos(t + rot), cy + r * math.sin(t + rot), r
  end
  screen.level(3)
  local px, py = at(0)
  for i = 1, 80 do
    local x, y, r = at(i / 16 * math.pi)
    if r > 29 then break end
    screen.move(px, py)
    screen.line(x, y)
    screen.stroke()
    px, py = x, y
  end
  -- the current value as a dot riding the spiral
  local x, y = at(util.linlin(0, params:get("mod") - 1, math.pi, 4.5 * math.pi, last))
  screen.level(15)
  screen.circle(x, y, 2)
  screen.fill()
  screen.move(0, 8)
  screen.text("fibonacci")
  screen.level(6)
  screen.move(0, 24)
  screen.text(a .. " " .. b)
  screen.move(0, 34)
  screen.text("mod " .. params:get("mod"))
  screen.level(4)
  screen.move(0, 62)
  screen.text("period " .. period .. "  " .. (count % math.max(period, 1)))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or ("seed " .. seed[1] .. "," .. seed[2]))
  screen.update()
end
