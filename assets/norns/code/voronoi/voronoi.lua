-- voronoi
-- a Portamax norns script
--
-- seven seeds drift, each claiming
-- the ground nearest to it. a beam
-- sweeps across the map; every
-- time it crosses into another
-- seed's cell, that seed sings.
--
-- E2 beam speed   E3 brightness
-- K2 scatter seeds   K3 beam dir
-- pads: plant a seed
-- (params: scale, root, seeds)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H = 128, 56
local BX, BY = 2, 2
local GW, GH = W // BX, H // BY
local ROWS = 14
local scale = {}
local seeds = {}
local beam = 0
local beam_dir = 1
local prev_owner = {}
local owners = {}
local frame = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function new_seed(x, y, deg)
  return {
    x = x or math.random(6, W - 6),
    y = y or math.random(6, H - 6),
    vx = (math.random() - 0.5) * 0.5,
    vy = (math.random() - 0.5) * 0.4,
    degree = deg or math.random(1, 14),
    glow = 0,
    cool = 0,
  }
end

local function scatter()
  seeds = {}
  local degs = { 1, 3, 5, 6, 8, 10, 12, 4, 9, 13 }
  for i = 1, params:get("seeds") do seeds[i] = new_seed(nil, nil, degs[i]) end
  prev_owner = {}
end

local function owner(x, y)
  local best, bd = 1, math.huge
  for i, s in ipairs(seeds) do
    local dx, dy = x - s.x, y - s.y
    local d = dx * dx + dy * dy
    if d < bd then best, bd = i, d end
  end
  return best
end

local function sing(i, y)
  local s = seeds[i]
  if not s or s.cool > 0 then return end
  engine.pan(util.linlin(0, W, -0.85, 0.85, beam))
  engine.pw(util.linlin(0, H, 0.15, 0.6, y))
  engine.amp(0.22)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(s.degree, 1, #scale)]))
  s.glow = 15
  s.cool = 5
end

local function step()
  frame = frame + 1
  for _, s in ipairs(seeds) do
    s.x = s.x + s.vx
    s.y = s.y + s.vy
    if s.x < 2 or s.x > W - 2 then s.vx = -s.vx end
    if s.y < 2 or s.y > H - 2 then s.vy = -s.vy end
    s.glow = math.max(0, s.glow - 0.5)
    s.cool = math.max(0, s.cool - 1)
  end
  beam = beam + beam_dir * params:get("beam")
  if beam >= W then beam = beam - W elseif beam < 0 then beam = beam + W end
  -- one note per frame at most: the first border the beam crosses
  local sung = false
  for r = 1, ROWS do
    local y = (r - 0.5) * H / ROWS
    local o = owner(beam, y)
    if prev_owner[r] and prev_owner[r] ~= o and not sung then
      sing(o, y)
      sung = true
    end
    prev_owner[r] = o
  end
  if frame % 2 == 0 then redraw() end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("VORONOI")
  params:add_option("scale", "scale", names, 12)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("seeds", "seeds", 3, 10, 7)
  params:set_action("seeds", function() scatter() end)
  params:add_control("beam", "beam speed", controlspec.new(0.2, 4, 'exp', 0, 1.2, 'px'))
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 2000, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.1, 4, 'exp', 0, 1.3, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.4)
  math.randomseed(os.time())
  build_scale()
  scatter()
  sing(1, 20)

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      if #seeds >= 10 then table.remove(seeds, 1) end
      local deg = 1
      for i, n in ipairs(scale) do if n <= msg.note then deg = i end end
      table.insert(seeds, new_seed(beam, math.random(6, H - 6), deg))
      sing(#seeds, 28)
    end
  end

  local mt = metro.init(step, 1 / 30)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("beam", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then scatter()
  elseif n == 3 then beam_dir = -beam_dir end
end

function redraw()
  screen.clear()
  for gy = 0, GH - 1 do
    local row = owners[gy] or {}
    owners[gy] = row
    for gx = 0, GW - 1 do
      row[gx] = owner(gx * BX + 1, gy * BY + 1)
    end
  end
  -- shade the cells that are ringing
  for gy = 0, GH - 1, 2 do
    for gx = (gy // 2) % 2, GW - 1, 2 do
      local s = seeds[owners[gy][gx]]
      if s and s.glow > 2 then
        screen.level(math.floor(s.glow / 4))
        screen.pixel(gx * BX, gy * BY)
        screen.fill()
      end
    end
  end
  -- borders
  screen.level(5)
  for gy = 0, GH - 1 do
    local row, below = owners[gy], owners[gy + 1]
    for gx = 0, GW - 1 do
      local o = row[gx]
      if (gx < GW - 1 and row[gx + 1] ~= o) or (below and below[gx] ~= o) then
        screen.pixel(gx * BX + 1, gy * BY + 1)
        screen.fill()
      end
    end
  end
  for _, s in ipairs(seeds) do
    screen.level(math.max(7, math.floor(s.glow)))
    screen.circle(s.x, s.y, 1.5)
    screen.fill()
  end
  screen.level(10)
  screen.move(beam, 0)
  screen.line(beam, H)
  screen.stroke()
  screen.level(15)
  screen.move(0, 63)
  screen.text("voronoi")
  screen.level(4)
  screen.move(127, 63)
  screen.text_right((beam_dir > 0 and ">" or "<") .. " " .. params:string("beam"))
  screen.update()
end
