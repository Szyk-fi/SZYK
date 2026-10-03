-- fireflies
-- a Portamax norns script
--
-- ten fireflies blink on their own
-- clocks. each one nudges the
-- others' phase toward its own, so
-- the swarm slowly falls into step.
-- every flash plays that fly's note.
--
-- E2 coupling   E3 brightness
-- K2 scatter phases   K3 pause
-- pads: wake a firefly
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 10
local flies = {}
local scale = {}
local paused = false
local sync = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), N + 4)
end

local function flash(i)
  local f = flies[i]
  f.glow = 15
  engine.pan((f.x - 64) / 70)
  engine.pw(0.3 + 0.4 * (i / N))
  engine.hz(MusicUtil.note_num_to_freq(scale[f.deg]))
end

local function scatter()
  for i = 1, N do
    flies[i] = flies[i] or {}
    local f = flies[i]
    f.x = 8 + math.random() * 112
    f.y = 14 + math.random() * 34
    f.phase = math.random() * 2 * math.pi
    f.w = 2 * math.pi * (0.45 + math.random() * 0.3) -- flashes per second
    f.deg = ({ 1, 3, 5, 2, 4, 6, 8, 3, 5, 7 })[i]
    f.glow = 0
  end
  flies[1].phase = 2 * math.pi - 0.05 -- the first one is about to blink
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("FIREFLIES")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("coupling", "coupling", controlspec.new(0, 3, 'lin', 0.05, 0.6, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'lin', 0, 0.9, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.amp(0.16)
  math.randomseed(os.time())
  build_scale()
  scatter()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local i = (msg.note % N) + 1
      flies[i].phase = 0
      flash(i)
    end
  end
  local dt = 1 / 30
  metro.init(function()
    if not paused then step(dt) end
    for _, f in ipairs(flies) do f.glow = math.max(0, f.glow - 1) end
    redraw()
  end, dt):start()
end

function step(dt)
  local k = params:get("coupling")
  local cx, sx = 0, 0
  for _, f in ipairs(flies) do cx = cx + math.cos(f.phase) sx = sx + math.sin(f.phase) end
  sync = math.sqrt(cx * cx + sx * sx) / N
  -- Kuramoto: each phase is pulled toward the others
  local pull = {}
  for i, f in ipairs(flies) do
    local s = 0
    for _, g in ipairs(flies) do s = s + math.sin(g.phase - f.phase) end
    pull[i] = k * s / N
  end
  for i, f in ipairs(flies) do
    f.phase = f.phase + (f.w + pull[i]) * dt
    if f.phase >= 2 * math.pi then
      f.phase = f.phase - 2 * math.pi
      flash(i)
    elseif f.phase < 0 then
      f.phase = f.phase + 2 * math.pi
    end
  end
end

function enc(n, d)
  if n == 2 then params:delta("coupling", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    for _, f in ipairs(flies) do f.phase = math.random() * 2 * math.pi end
    flies[1].phase = 2 * math.pi - 0.05
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  for _, f in ipairs(flies) do
    screen.level(math.max(1, f.glow))
    screen.circle(f.x, f.y, f.glow > 8 and 2 or 1)
    screen.fill()
    if f.glow > 10 then
      screen.level(f.glow - 10)
      screen.circle(f.x, f.y, 5)
      screen.stroke()
    end
  end
  -- grass
  screen.level(2)
  for x = 0, 127, 3 do
    screen.move(x, 54)
    screen.line(x + 1, 50 - (x * 7 % 4))
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "fireflies (paused)" or "fireflies")
  screen.level(4)
  screen.move(0, 62)
  screen.text("couple " .. params:string("coupling"))
  screen.move(127, 62)
  screen.text_right(string.format("sync %d%%", math.floor(sync * 100)))
  screen.update()
end
