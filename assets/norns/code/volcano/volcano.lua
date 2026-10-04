-- volcano
-- a Portamax norns script
--
-- magma pressure builds under the
-- mountain. the rumble rises in
-- pitch and grows busier as the
-- gauge fills, until the summit
-- blows: lava bombs fly out, each
-- a note pitched by its speed, and
-- the pressure starts over.
--
-- E2 build rate   E3 eruption size
-- K2 erupt now   K3 pause
-- pads: a tremor
-- (params: scale, root, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local pressure = 0.15
local paused = false
local bombs = {}
local quake = 0
local eruptions = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 20)
end

local function rumble()
  local deg = 1 + math.floor(pressure * 6)
  engine.pan((math.random() - 0.5) * 0.4)
  engine.pw(0.5)
  engine.cutoff(params:get("tone") * (0.3 + pressure * 0.5))
  engine.release(0.6 + pressure)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg] - 12))
  quake = 3
end

local function erupt()
  eruptions = eruptions + 1
  pressure = 0
  quake = 10
  local n = params:get("size")
  clock.run(function()
    engine.cutoff(params:get("tone") * 0.4)
    engine.release(3)
    engine.hz(MusicUtil.note_num_to_freq(scale[1] - 12))
    for i = 1, n do
      local v = 0.6 + math.random() * 1.6
      local dir = (math.random() - 0.5) * 2
      table.insert(bombs, { x = 64, y = 22, vx = dir * v, vy = -v * 1.6 })
      engine.pan(dir * 0.8)
      engine.pw(0.2 + math.random() * 0.3)
      engine.cutoff(params:get("tone"))
      engine.release(0.5 + v * 0.4)
      engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(math.floor(v * 8), 4, #scale)]))
      clock.sleep(0.05 + math.random() * 0.12)
    end
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("VOLCANO")
  params:add_option("scale", "scale", names, 3)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("rate", "build rate", controlspec.new(0.01, 0.3, 'exp', 0, 0.06, '/s'))
  params:add_number("size", "eruption size", 3, 24, 12)
  params:add_control("tone", "tone", controlspec.new(500, 8000, 'exp', 0, 3000, 'hz'))
  params:default()
  engine.amp(0.24)
  math.randomseed(os.time())
  build_scale()
  rumble()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then pressure = math.min(1, pressure + 0.08) rumble() end
  end
  -- the rumble: more often, and higher, as pressure builds
  clock.run(function()
    while true do
      clock.sleep(0.9 - pressure * 0.7)
      if not paused then rumble() end
    end
  end)
  local dt = 1 / 30
  metro.init(function()
    if not paused then
      pressure = pressure + params:get("rate") * dt
      if pressure >= 1 then erupt() end
    end
    for i = #bombs, 1, -1 do
      local b = bombs[i]
      b.x, b.y, b.vy = b.x + b.vx, b.y + b.vy, b.vy + 0.08
      if b.y > 56 then table.remove(bombs, i) end
    end
    quake = math.max(0, quake - 1)
    redraw()
  end, dt):start()
end

function enc(n, d)
  if n == 2 then params:delta("rate", d)
  elseif n == 3 then params:delta("size", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then erupt()
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  local s = quake > 0 and math.random(-1, 1) or 0
  -- the mountain
  screen.level(6)
  screen.move(8 + s, 54)
  screen.line(56 + s, 22)
  screen.line(72 + s, 22)
  screen.line(120 + s, 54)
  screen.stroke()
  -- magma rising in the vent
  screen.level(math.floor(4 + pressure * 11))
  local top = 54 - pressure * 30
  screen.rect(62 + s, top, 4, 54 - top)
  screen.fill()
  for _, b in ipairs(bombs) do
    screen.level(15)
    screen.rect(b.x - 1, b.y - 1, 2, 2)
    screen.fill()
  end
  -- pressure gauge
  screen.level(3)
  screen.rect(122, 14, 4, 40)
  screen.stroke()
  screen.level(pressure > 0.8 and 15 or 8)
  screen.rect(123, 54 - pressure * 40, 2, pressure * 40)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "volcano (paused)" or "volcano")
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.format("pressure %d%%", math.floor(pressure * 100)))
  screen.move(120, 62)
  screen.text_right("eruptions " .. eruptions)
  screen.update()
end
