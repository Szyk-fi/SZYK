-- koi
-- a Portamax norns script
--
-- koi glide around a still pond,
-- deep and dim, then drift up to
-- the surface. where one breaks the
-- water a ripple spreads and a note
-- sounds, with a softer echo after.
-- big fish sing low, small fish high.
--
-- E2 number of fish   E3 swim speed
-- K2 scatter   K3 pause
-- pads: scatter food (fish rise)
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local fish = {}
local ripples = {}
local pads = { { 22, 20, 6 }, { 104, 46, 7 }, { 92, 18, 4 } }
local scale = {}
local paused = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 18)
end

local function new_fish()
  return { x = math.random(10, 118), y = math.random(16, 54), a = math.random() * 6.28,
    size = math.random(3, 6), depth = math.random(), dz = -0.006 - math.random() * 0.006 }
end

local function surface(f)
  table.insert(ripples, { x = f.x, y = f.y, r = 1 })
  local deg = math.floor(util.linlin(0, 128, 1, 8, f.x)) + (6 - f.size) * 2
  local n = scale[util.clamp(deg, 1, #scale)]
  engine.amp(0.24) engine.pw(0.4)
  engine.pan(util.linlin(0, 128, -0.7, 0.7, f.x))
  engine.hz(MusicUtil.note_num_to_freq(n))
  clock.run(function()
    clock.sleep(0.35 + math.random() * 0.3)
    engine.amp(0.09)
    engine.hz(MusicUtil.note_num_to_freq(n + 12))
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("KOI")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 43, 67, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("count", "fish", 1, 9, 5)
  params:add_control("speed", "swim speed", controlspec.new(0.2, 2.5, 'exp', 0, 0.8, 'x'))
  params:add_control("release", "release", controlspec.new(0.3, 5, 'exp', 0, 2.4, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(1500)
  math.randomseed(os.time())
  build_scale()
  for i = 1, params:get("count") do fish[i] = new_fish() end
  fish[1].depth = 0.02
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local f = fish[(msg.note - 60) % #fish + 1]
      f.depth = 0.01
      surface(f)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then swim() end
      redraw()
    end
  end)
end

function swim()
  while #fish < params:get("count") do fish[#fish + 1] = new_fish() end
  while #fish > params:get("count") do table.remove(fish) end
  local v = params:get("speed")
  for _, f in ipairs(fish) do
    f.a = f.a + (math.random() - 0.5) * 0.2
    -- turn back from the edge of the pond
    local cx, cy = 64 - f.x, 36 - f.y
    if math.abs(cx) > 50 or math.abs(cy) > 16 then f.a = f.a + 0.1 * ((math.sin(math.atan(cy, cx) - f.a) > 0) and 1 or -1) end
    f.x = f.x + math.cos(f.a) * v * 0.5
    f.y = f.y + math.sin(f.a) * v * 0.3
    local before = f.depth
    f.depth = util.clamp(f.depth + f.dz * v, 0, 1)
    if before > 0 and f.depth == 0 then surface(f) end
    if f.depth == 0 or f.depth == 1 then f.dz = -f.dz end
    if f.depth > 0.6 and math.random() < 0.01 then f.dz = -math.abs(f.dz) end
  end
  for i = #ripples, 1, -1 do
    ripples[i].r = ripples[i].r + 0.5
    if ripples[i].r > 18 then table.remove(ripples, i) end
  end
end

function enc(n, d)
  if n == 2 then params:delta("count", d)
  elseif n == 3 then params:delta("speed", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    for _, f in ipairs(fish) do f.a = math.random() * 6.28 f.dz = math.abs(f.dz) end
    table.insert(ripples, { x = 64, y = 36, r = 1 })
    engine.amp(0.2) engine.hz(MusicUtil.note_num_to_freq(scale[1] - 12))
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  for _, f in ipairs(fish) do
    local lvl = math.floor(util.linlin(0, 1, 14, 2, f.depth))
    screen.level(lvl)
    screen.circle(f.x, f.y, f.size * 0.6) screen.fill()
    screen.move(f.x - math.cos(f.a) * f.size * 0.5, f.y - math.sin(f.a) * f.size * 0.5)
    screen.line(f.x - math.cos(f.a + 0.3) * f.size * 1.6, f.y - math.sin(f.a + 0.3) * f.size * 1.6)
    screen.stroke()
  end
  for _, p in ipairs(pads) do
    screen.level(4)
    screen.circle(p[1], p[2], p[3]) screen.fill()
    screen.level(0)
    screen.move(p[1], p[2]) screen.line(p[1] + p[3], p[2] - 2) screen.stroke()
  end
  for _, r in ipairs(ripples) do
    screen.level(math.max(1, 12 - math.floor(r.r * 0.6)))
    screen.circle(r.x, r.y, r.r) screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "koi (paused)" or "koi")
  screen.level(4)
  screen.move(0, 62)
  screen.text(#fish .. " fish")
  screen.move(127, 62)
  screen.text_right("swim " .. params:string("speed"))
  screen.update()
end
