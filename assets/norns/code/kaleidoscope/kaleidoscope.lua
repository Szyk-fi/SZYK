-- kaleidoscope
-- a Portamax norns script
--
-- coloured shards tumble between
-- mirrors. they drift in and out
-- from the centre; when one falls
-- all the way in, its reflections
-- meet and a chord rings out.
-- touching the rim, they glint.
--
-- E2 mirrors   E3 brightness
-- K2 new shards   K3 reverse spin
-- pads: drop a shard from the rim
-- (params: scale, root, shards)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CX, CY, RMAX = 64, 31, 29
local scale = {}
local shards = {}
local spin = 1
local rings = {}
local t = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function new_shard(r, deg)
  return {
    r = r or math.random() * RMAX,
    vr = (0.25 + math.random() * 0.45) * (math.random() < 0.5 and -1 or 1),
    a = math.random() * math.pi,
    va = 0.005 + math.random() * 0.02,
    size = 3 + math.random() * 5,
    degree = deg or math.random(1, 7),
    glow = 0,
  }
end

local function new_shards()
  shards = {}
  for i = 1, params:get("shards") do shards[i] = new_shard() end
  -- one starts on its way in, so the first chord comes soon
  shards[1].r, shards[1].vr = 4, -0.6
end

local function chord(deg)
  -- a triad stacked in scale steps, strummed outward from the centre
  clock.run(function()
    local tones = { deg, deg + 2, deg + 4, deg + 7 }
    local pans = { 0, -0.6, 0.6, 0 }
    for i, d in ipairs(tones) do
      engine.pan(pans[i])
      engine.pw(0.35 + i * 0.08)
      engine.amp(0.16)
      engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(d, 1, #scale)]))
      clock.sleep(0.035)
    end
  end)
  table.insert(rings, { r = 1 })
end

local function glint(s)
  engine.pan((math.random() - 0.5) * 1.6)
  engine.pw(0.12)
  engine.amp(0.07)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(s.degree + 14, 1, #scale)]))
end

local function step()
  t = t + 1
  for _, s in ipairs(shards) do
    s.r = s.r + s.vr
    s.a = s.a + s.va * spin
    s.glow = math.max(0, s.glow - 1)
    if s.r <= 0.5 and s.vr < 0 then
      s.r, s.vr = 0.5, -s.vr
      s.glow = 15
      chord(s.degree)
      -- the next pass through the centre moves the harmony on
      s.degree = (s.degree + 3) % 7 + 1
    elseif s.r >= RMAX and s.vr > 0 then
      s.r, s.vr = RMAX, -s.vr
      s.glow = 8
      if math.random() < 0.5 then glint(s) end
    end
  end
  for i = #rings, 1, -1 do
    rings[i].r = rings[i].r + 1.2
    if rings[i].r > RMAX then table.remove(rings, i) end
  end
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("KALEIDOSCOPE")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("mirrors", "mirrors", 2, 8, 6)
  params:add_number("shards", "shards", 1, 8, 4)
  params:set_action("shards", function() new_shards() end)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 2400, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.2, 5, 'exp', 0, 2.2, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.1)
  math.randomseed(os.time())
  build_scale()
  new_shards()
  chord(1)

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local deg = 1
      for i, n in ipairs(scale) do if n <= msg.note then deg = i end end
      local s = new_shard(RMAX, ((deg - 1) % 7) + 1)
      s.vr = -0.9
      if #shards >= 8 then table.remove(shards, 1) end
      table.insert(shards, s)
      engine.pan(0)
      engine.amp(0.15)
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
    end
  end

  local mt = metro.init(step, 1 / 30)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("mirrors", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_shards()
  elseif n == 3 then spin = -spin end
end

local function polar(r, a)
  return CX + math.cos(a) * r, CY + math.sin(a) * r
end

function redraw()
  screen.clear()
  local k = params:get("mirrors")
  local wedge = 2 * math.pi / k
  -- the mirror lines
  screen.level(1)
  for j = 0, k - 1 do
    local x, y = polar(RMAX + 2, j * wedge + t * 0.002 * spin)
    screen.move(CX, CY)
    screen.line(x, y)
    screen.stroke()
  end
  for _, s in ipairs(shards) do
    screen.level(math.max(4, s.glow))
    -- fold the shard's angle into one wedge, then reflect it
    -- into every other wedge, mirrored every second time
    local a = s.a % wedge
    for j = 0, k - 1 do
      for m = 0, 1 do
        local base = j * wedge + t * 0.002 * spin
        local aa = m == 0 and (base + a) or (base + wedge - a)
        local x1, y1 = polar(s.r, aa)
        local x2, y2 = polar(s.r + s.size, aa + 0.15 * (m == 0 and 1 or -1))
        local x3, y3 = polar(s.r + s.size * 0.6, aa - 0.12 * (m == 0 and 1 or -1))
        screen.move(x1, y1)
        screen.line(x2, y2)
        screen.line(x3, y3)
        screen.close()
        if s.glow > 6 then screen.fill() else screen.stroke() end
      end
    end
  end
  for _, rg in ipairs(rings) do
    screen.level(math.max(1, math.floor(12 - rg.r / 3)))
    screen.circle(CX, CY, rg.r)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("kaleido")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right(k .. "x")
  screen.update()
end
