-- starfield
-- a Portamax norns script
--
-- flying through a field of stars.
-- bright stars that pass close by
-- ring out; the nearer the star,
-- the louder and brighter the note.
-- where it passes sets the pitch.
--
-- E2 speed   E3 star count
-- K2 hyperjump   K3 drift / fly
-- pads: a passing comet
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local stars = {}
local notes = {}
local flying = true
local boost = 0
local rung = 0

local function build_scale()
  notes = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 14)
end

local function new_star(far)
  return {
    x = math.random() * 2 - 1,
    y = math.random() * 2 - 1,
    z = far and (0.8 + math.random() * 0.2) or (0.1 + math.random() * 0.9),
    bright = math.random() < 0.35,
  }
end

local function ring(s)
  -- angle around the centre picks the degree, height picks the octave
  local ang = (math.atan(s.y, s.x) + math.pi) / (2 * math.pi)
  local deg = util.clamp(math.floor(ang * #notes) + 1, 1, #notes)
  local near = util.clamp(1 - math.sqrt(s.x * s.x + s.y * s.y) / 1.4, 0, 1)
  engine.amp(0.12 + 0.2 * near)
  engine.cutoff(800 + 3200 * near)
  engine.pan(util.clamp(s.x, -1, 1))
  engine.hz(MusicUtil.note_num_to_freq(notes[deg]))
  rung = 6
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("STARFIELD")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "speed", controlspec.new(0.2, 3, 'lin', 0, 1, 'x'))
  params:add_number("count", "star count", 8, 60, 28)
  params:add_control("release", "release", controlspec.new(0.2, 4, 'exp', 0, 1.8, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.35)
  build_scale()
  math.randomseed(os.time())
  for i = 1, params:get("count") do stars[i] = new_star(false) end
  stars[1].z, stars[1].bright = 0.17, true
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local s = new_star(false)
      s.x, s.y, s.z, s.bright = (msg.note % 12) / 6 - 1, 0.3, 0.16, true
      table.insert(stars, s)
      ring(s)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      step()
      redraw()
    end
  end)
end

function step()
  local v = (flying and 0.012 or 0.002) * params:get("speed") * (1 + boost)
  boost = boost * 0.93
  rung = math.max(0, rung - 1)
  for i = #stars, 1, -1 do
    local s = stars[i]
    local before = s.z
    s.z = s.z - v
    if before >= 0.15 and s.z < 0.15 and s.bright then ring(s) end
    if s.z <= 0.02 then
      if #stars > params:get("count") then table.remove(stars, i) else stars[i] = new_star(true) end
    end
  end
  while #stars < params:get("count") do stars[#stars + 1] = new_star(true) end
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("count", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then boost = 6
  elseif n == 3 then flying = not flying end
  redraw()
end

function redraw()
  screen.clear()
  for _, s in ipairs(stars) do
    local px = 64 + s.x / s.z * 18
    local py = 34 + s.y / s.z * 12
    if px >= 0 and px < 128 and py > 10 and py < 54 then
      local lvl = math.floor(util.linlin(0.02, 1, 15, 1, s.z))
      screen.level(s.bright and lvl or math.max(1, lvl // 2))
      if s.z < 0.25 and s.bright then
        screen.circle(px, py, 1.5)
        screen.fill()
      else
        screen.pixel(px, py)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(flying and "starfield" or "starfield (drift)")
  screen.level(rung > 0 and 15 or 2)
  screen.circle(124, 5, 2)
  screen.fill()
  screen.level(4)
  screen.move(0, 62)
  screen.text("speed " .. params:string("speed"))
  screen.move(127, 62)
  screen.text_right(params:get("count") .. " stars")
  screen.update()
end
