-- clockwork
-- a Portamax norns script
--
-- a train of meshed gears. meshed
-- teeth all move at the same rate,
-- so each gear turns at a speed set
-- by its tooth count. marked teeth
-- ring as they pass the top: the
-- tooth ratios become a polyrhythm.
--
-- E2 speed   E3 marks per gear
-- K2 new gear train   K3 clutch
-- pads: strike a gear
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SIZES = { 8, 9, 10, 12, 15, 16, 18, 20, 24 }
local gears = {}
local scale = {}
local teeth = 0
local running = true

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 21)
end

local function new_train()
  gears = {}
  local x, y, up = 10, 36, 1
  for i = 1, 5 do
    local t = SIZES[math.random(#SIZES)]
    local r = t * 0.55
    if i > 1 then
      local prev = gears[i - 1]
      local d = prev.r + r + 1
      local a = math.rad(math.random(15, 40)) * up
      x, y = prev.x + d * math.cos(a), util.clamp(prev.y + d * math.sin(a), 16 + r, 54 - r)
      up = -up
    else
      x = 6 + r
    end
    if x + r > 127 then break end
    gears[i] = { t = t, r = r, x = x, y = y, last = 0, hit = 0 }
  end
  table.sort(gears, function(a, b) return a.x < b.x end)
end

local function ring(i, m)
  local g = gears[i]
  -- bigger gear, lower voice; each mark sings its own degree
  local base = util.round(util.linlin(8, 24, 14, 1, g.t))
  local n = scale[util.clamp(base + (m % 3) * 2, 1, #scale)]
  engine.amp(0.12 + g.t / 120)
  engine.pan(util.linlin(0, 128, -0.8, 0.8, g.x))
  engine.pw(0.1 + g.t / 60)
  engine.hz(MusicUtil.note_num_to_freq(n))
  g.hit = 6
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CLOCKWORK")
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "teeth / sec", controlspec.new(1, 16, 'exp', 0, 6, ''))
  params:add_number("marks", "marks per gear", 1, 4, 3)
  params:add_control("release", "release", controlspec.new(0.1, 2, 'lin', 0, 0.7, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2600)
  math.randomseed(os.time())
  build_scale()
  new_train()
  ring(1, 0)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then ring((msg.note - 60) % #gears + 1, msg.note) end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if running then
        teeth = teeth + params:get("speed") / 30
        local marks = params:get("marks")
        for i, g in ipairs(gears) do
          local k = math.floor(teeth * marks / g.t)
          if k ~= g.last then g.last = k ring(i, k % marks) end
        end
      end
      for _, g in ipairs(gears) do g.hit = math.max(0, g.hit - 1) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("marks", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    new_train()
    for _, g in ipairs(gears) do g.last = math.floor(teeth * params:get("marks") / g.t) end
    ring(1, 0)
  elseif n == 3 then running = not running end
  redraw()
end

function redraw()
  screen.clear()
  local marks = params:get("marks")
  for i, g in ipairs(gears) do
    -- neighbours turn opposite ways; a tooth is 2*pi/t of a turn
    local dir = (i % 2 == 1) and 1 or -1
    local ang = dir * teeth * 2 * math.pi / g.t
    screen.level(g.hit > 0 and 15 or 5)
    screen.circle(g.x, g.y, g.r - 1)
    screen.stroke()
    for k = 0, g.t - 1 do
      local a = ang + k * 2 * math.pi / g.t - math.pi / 2
      local marked = (k * marks) % g.t < marks
      screen.level(marked and 15 or 4)
      screen.move(g.x + math.cos(a) * (g.r - 1), g.y + math.sin(a) * (g.r - 1))
      screen.line(g.x + math.cos(a) * (g.r + 1), g.y + math.sin(a) * (g.r + 1))
      screen.stroke()
    end
    screen.level(3)
    screen.circle(g.x, g.y, 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(running and "clockwork" or "clockwork (clutch)")
  screen.level(4)
  screen.move(0, 62)
  local ts = {}
  for i, g in ipairs(gears) do ts[i] = g.t end
  screen.text(table.concat(ts, ":"))
  screen.move(127, 62)
  screen.text_right(params:string("speed") .. " t/s")
  screen.update()
end
