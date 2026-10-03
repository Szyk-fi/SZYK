-- logistic
-- a Portamax norns script
--
-- x becomes r * x * (1 - x), over
-- and over. low r settles on one
-- note; turn it up and the tune
-- splits into two, four, eight,
-- then chaos, with calm windows
-- hiding inside. the bifurcation
-- diagram shows where you are.
--
-- E2 r (calm to chaos)   E3 speed
-- K2 nudge x   K3 pause
-- pads: root note
-- (params: scale, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local R0, R1 = 2.8, 4.0
local DIVS = { 1 / 4, 1 / 3, 1 / 2 }
local x = 0.3
local diagram = {}
local trail = {}
local root = 45
local scale = {}
local paused = false

local function build_diagram()
  -- for each screen column of r: run past the transient, then
  -- mark which pixel rows the orbit visits
  for c = 0, 127 do
    local r = R0 + (R1 - R0) * c / 127
    local v = 0.5
    for _ = 1, 150 do v = r * v * (1 - v) end
    local seen, col = {}, {}
    for _ = 1, 48 do
      v = r * v * (1 - v)
      local py = 52 - math.floor(v * 38)
      if not seen[py] then seen[py] = true col[#col + 1] = py end
    end
    diagram[c] = col
  end
end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 15)
end

local function tick()
  local r = params:get("r")
  x = r * x * (1 - x)
  if x <= 0 or x >= 1 or x ~= x then x = 0.3 end
  local deg = util.clamp(math.floor(x * #scale) + 1, 1, #scale)
  engine.amp(util.linlin(0, 1, 0.18, 0.3, x))
  engine.pan((x - 0.5) * 1.2)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  table.insert(trail, 1, x)
  if #trail > 12 then table.remove(trail) end
end

function init()
  build_diagram()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LOGISTIC")
  params:add_control("r", "r", controlspec.new(R0, R1, 'lin', 0.001, 3.2, ''))
  params:add_option("speed", "speed", { "1/16", "1/8t", "1/8" }, 1)
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_control("release", "release", controlspec.new(0.05, 2, 'exp', 0, 0.45, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:add_control("cutoff", "cutoff", controlspec.new(300, 8000, 'exp', 0, 2000, 'hz'))
  params:set_action("cutoff", function(v) engine.cutoff(v) end)
  params:default()
  engine.pw(0.4)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 15 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then tick() end
      redraw()
      clock.sync(DIVS[params:get("speed")])
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("r", d * 0.5)
  elseif n == 3 then params:delta("speed", -d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then x = math.random() * 0.8 + 0.1
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local rc = math.floor((params:get("r") - R0) / (R1 - R0) * 127 + 0.5)
  for c = 0, 127 do
    screen.level(c == rc and 12 or 3)
    for _, py in ipairs(diagram[c]) do
      screen.pixel(c, py)
    end
    screen.fill()
  end
  -- where r sits, and the last few values of x on it
  screen.level(5)
  screen.move(rc + 0.5, 13)
  screen.line(rc + 0.5, 15)
  screen.stroke()
  for i, v in ipairs(trail) do
    screen.level(math.max(2, 15 - i))
    screen.circle(rc + 0.5, 52 - v * 38, i == 1 and 2 or 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("logistic")
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.format("r %.3f  x %.2f", params:get("r"), x))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or params:string("speed"))
  screen.update()
end
