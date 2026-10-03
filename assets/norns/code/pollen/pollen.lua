-- pollen
-- a Portamax norns script
--
-- grains of pollen ride a slow
-- breeze across a meadow. when a
-- grain drifts into a flower the
-- flower hums its note.
--
-- E2 breeze   E3 brightness
-- K2 new meadow   K3 gust
-- pads: a flower releases pollen
-- (params: scale, root, grains,
--  release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local grains = {}
local flowers = {}
local t = 0
local gust = 0
local quiet = 0
local W, H = 128, 54

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function new_meadow()
  flowers = {}
  local n = 6
  for i = 1, n do
    flowers[i] = {
      x = 10 + (i - 1) * (108 / (n - 1)) + math.random(-4, 4),
      y = 12 + math.random() * 34,
      r = 5 + math.random() * 2,
      degree = ((i * 3) % #scale) + 1,
      glow = 0,
      cool = 0,
    }
  end
end

local function spawn(x, y)
  return { x = x or math.random() * W, y = y or math.random() * H, inside = {} }
end

local function set_grains(n)
  while #grains < n do table.insert(grains, spawn()) end
  while #grains > n do table.remove(grains) end
end

local function bloom(f, i)
  if f.cool > 0 then return end
  local note = scale[util.clamp(f.degree, 1, #scale)]
  engine.pan(util.linlin(0, W, -0.8, 0.8, f.x))
  engine.pw(util.linlin(0, H, 0.6, 0.2, f.y))
  engine.amp(0.18)
  engine.hz(MusicUtil.note_num_to_freq(note))
  f.glow = 15
  f.cool = 6
  quiet = 0
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("POLLEN")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("grains", "grains", 4, 60, 30)
  params:set_action("grains", set_grains)
  params:add_control("breeze", "breeze", controlspec.new(0.1, 3, 'exp', 0, 0.9, 'x'))
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.2, 5, 'exp', 0, 2.4, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_meadow()
  set_grains(params:get("grains"))
  -- the first flower opens straight away
  bloom(flowers[1], 1)

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local f = flowers[(msg.note % #flowers) + 1]
      engine.pan(util.linlin(0, W, -0.8, 0.8, f.x))
      engine.amp(0.2)
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
      f.glow = 15
      for _ = 1, 4 do
        local g = grains[math.random(1, #grains)]
        if g then g.x, g.y = f.x + math.random(-6, 6), f.y + math.random(-6, 6) end
      end
    end
  end

  local mt = metro.init(step, 1 / 30)
  mt:start()
end

local function flow(x, y)
  -- a slowly turning field of gentle eddies; it repeats exactly
  -- across the screen edges so grains don't pile up at the seams
  local a = math.sin(x * 2 * math.pi / W * 2 + t * 0.3) + math.cos(y * 2 * math.pi / H * 2 - t * 0.21) * 1.3
  return math.cos(a), math.sin(a) * 0.6
end

function step()
  t = t + 1 / 30
  local k = params:get("breeze") * (1 + gust)
  gust = gust * 0.96
  -- a becalmed meadow still hums now and then
  quiet = quiet + 1
  if quiet > 120 then bloom(flowers[math.random(1, #flowers)]) end
  for _, f in ipairs(flowers) do
    f.glow = math.max(0, f.glow - 0.6)
    f.cool = math.max(0, f.cool - 1)
  end
  for _, g in ipairs(grains) do
    local fx, fy = flow(g.x, g.y)
    g.x = (g.x + (fx * 0.6 + 0.25) * k + (math.random() - 0.5) * 0.8) % W
    g.y = (g.y + fy * 0.6 * k + (math.random() - 0.5) * 0.8) % H
    for i, f in ipairs(flowers) do
      local dx, dy = g.x - f.x, g.y - f.y
      local now = dx * dx + dy * dy < f.r * f.r
      if now and not g.inside[i] then bloom(f, i) end
      g.inside[i] = now
    end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("breeze", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    new_meadow()
    bloom(flowers[math.random(1, #flowers)])
  elseif n == 3 then
    gust = 2.5
  end
end

function redraw()
  screen.clear()
  -- flowers: a centre and five petals
  for _, f in ipairs(flowers) do
    local lv = math.floor(3 + f.glow * 0.8)
    screen.level(util.clamp(lv, 1, 15))
    for p = 0, 4 do
      local a = p * 2 * math.pi / 5 + t * 0.2
      screen.circle(f.x + math.cos(a) * 3.5, f.y + math.sin(a) * 3.5, 1.5)
      screen.stroke()
    end
    screen.level(util.clamp(math.floor(6 + f.glow * 0.6), 1, 15))
    screen.circle(f.x, f.y, 1.5)
    screen.fill()
  end
  -- grains
  screen.level(12)
  for _, g in ipairs(grains) do
    screen.pixel(g.x, g.y)
    screen.fill()
  end
  -- grass line
  screen.level(2)
  screen.move(0, 56)
  screen.line(127, 56)
  screen.stroke()
  screen.level(15)
  screen.move(0, 63)
  screen.text(gust > 0.3 and "pollen ~gust~" or "pollen")
  screen.level(4)
  screen.move(127, 63)
  screen.text_right("breeze " .. params:string("breeze"))
  screen.update()
end
