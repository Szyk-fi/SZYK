-- ripples
-- a Portamax norns script
--
-- two drops keep rippling a pond.
-- twelve lily pads float on it.
-- where the two wave trains arrive
-- in step they reinforce, and those
-- lilies sing at each crest; where
-- they cancel, the lilies stay still.
--
-- E2 wavelength   E3 drop spacing
-- K2 new drops   K3 pause
-- pads: a stone (plop)
-- (params: scale, root, threshold)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local drops = { { x = 50, y = 30 }, { x = 78, y = 30 } }
local lilies = {}
local scale = {}
local paused = false
local t = 0
local OMEGA = 2 * math.pi * 0.8

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 12)
end

local function place_drops(gap)
  local cx, cy = 64 + math.random(-12, 12), 22 + math.random(0, 16)
  drops[1].x, drops[1].y = cx - gap / 2, cy
  drops[2].x, drops[2].y = cx + gap / 2, cy
end

local function plop(note)
  engine.pw(0.5)
  engine.release(2.5)
  engine.pan(0)
  engine.hz(MusicUtil.note_num_to_freq(note))
end

-- envelope (how strongly the two waves agree) and travelling phase at a point
local function field(x, y)
  local k = 2 * math.pi / params:get("wavelength")
  local r1 = math.sqrt((x - drops[1].x) ^ 2 + (y - drops[1].y) ^ 2)
  local r2 = math.sqrt((x - drops[2].x) ^ 2 + (y - drops[2].y) ^ 2)
  return math.cos(k * (r1 - r2) / 2), k * (r1 + r2) / 2 - OMEGA * t
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("RIPPLES")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("wavelength", "wavelength", controlspec.new(6, 40, 'lin', 0.5, 16, 'px'))
  params:add_number("gap", "drop spacing", 4, 80, 28)
  params:set_action("gap", function(g) place_drops(g) end)
  params:add_control("threshold", "sing above", controlspec.new(0.3, 0.98, 'lin', 0.01, 0.8, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 2400, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.2)
  math.randomseed(os.time())
  build_scale()
  for i = 1, 12 do lilies[i] = { x = 6 + (i - 1) * 10.5, y = 46 + (i % 2) * 4, last = 0, glow = 0 } end
  for _, l in ipairs(lilies) do
    local _, ph = field(l.x, l.y)
    l.last = math.floor(ph / (2 * math.pi))
  end
  plop(params:get("root") - 12)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then plop(msg.note - 12) end
  end
  metro.init(function()
    if not paused then step(1 / 30) end
    redraw()
  end, 1 / 30):start()
end

function step(dt)
  t = t + dt
  local th = params:get("threshold")
  for i, l in ipairs(lilies) do
    l.glow = math.max(0, l.glow - 1)
    local env, ph = field(l.x, l.y)
    local crest = math.floor(ph / (2 * math.pi))
    if crest ~= l.last then
      l.last = crest
      if math.abs(env) > th then
        engine.pan((l.x - 64) / 70)
        engine.pw(0.25)
        engine.release(0.6 + math.abs(env))
        engine.hz(MusicUtil.note_num_to_freq(scale[i]))
        l.glow = 15
      end
    end
  end
end

function enc(n, d)
  if n == 2 then params:delta("wavelength", d)
  elseif n == 3 then params:delta("gap", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then place_drops(params:get("gap")) plop(params:get("root") - 12)
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  for y = 12, 54, 3 do
    for x = 1, 127, 3 do
      local env, ph = field(x, y)
      local v = env * math.cos(ph)
      if v > 0.3 then
        screen.level(math.floor(v * 7))
        screen.pixel(x, y)
        screen.fill()
      end
    end
  end
  screen.level(15)
  for _, d in ipairs(drops) do screen.circle(d.x, d.y, 2) screen.fill() end
  for _, l in ipairs(lilies) do
    screen.level(math.max(3, l.glow))
    screen.circle(l.x, l.y, 3)
    if l.glow > 0 then screen.fill() else screen.stroke() end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "ripples (paused)" or "ripples")
  screen.level(4)
  screen.move(0, 62)
  screen.text("wave " .. params:string("wavelength"))
  screen.move(127, 62)
  screen.text_right("gap " .. params:get("gap"))
  screen.update()
end
