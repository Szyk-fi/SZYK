-- magnets
-- a Portamax norns script
--
-- eight iron filings drift between
-- two magnets. the left one pulls,
-- the right one pushes (or both
-- pull). whenever two filings clack
-- together, or one hits a magnet,
-- it rings.
--
-- E2 move left magnet   E3 move right
-- K2 flip right polarity   K3 pause
-- pads: kick the filings
-- (params: scale, root, strength)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local mags = { { x = 40, y = 34, pol = 1 }, { x = 88, y = 34, pol = -1 } }
local bits = {}
local scale = {}
local paused = false
local rings = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function ring(x, y, deg, bright)
  local note = scale[util.clamp(deg, 1, #scale)]
  engine.pan((x - 64) / 64)
  engine.pw(bright and 0.15 or 0.5)
  engine.release(bright and 1.6 or 0.7)
  engine.hz(MusicUtil.note_num_to_freq(note + (bright and 12 or 0)))
  table.insert(rings, { x = x, y = y, r = 1 })
end

local function kick(v)
  for _, b in ipairs(bits) do
    b.vx = b.vx + (math.random() - 0.5) * v
    b.vy = b.vy + (math.random() - 0.5) * v
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("MAGNETS")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("strength", "strength", controlspec.new(5, 120, 'exp', 0, 40, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 2200, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.22)
  math.randomseed(os.time())
  build_scale()
  for i = 1, 8 do
    bits[i] = { x = 10 + i * 13, y = 16 + (i % 3) * 14, vx = (math.random() - 0.5) * 3, vy = (math.random() - 0.5) * 3, cool = 0 }
  end
  ring(64, 34, 1, false)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then kick(4) end
  end
  metro.init(function()
    if not paused then step() end
    for i = #rings, 1, -1 do
      rings[i].r = rings[i].r + 1
      if rings[i].r > 9 then table.remove(rings, i) end
    end
    redraw()
  end, 1 / 30):start()
end

function step()
  local k = params:get("strength")
  for _, b in ipairs(bits) do
    b.cool = math.max(0, b.cool - 1)
    for mi, m in ipairs(mags) do
      local dx, dy = m.x - b.x, m.y - b.y
      local d2 = math.max(dx * dx + dy * dy, 16)
      local d = math.sqrt(d2)
      local f = m.pol * k / d2
      b.vx, b.vy = b.vx + f * dx / d, b.vy + f * dy / d
      if d < 5 and b.cool == 0 then
        -- struck the magnet: bounce off and ring high
        b.vx, b.vy = -b.vx - dx / d, -b.vy - dy / d
        b.cool = 8
        ring(b.x, b.y, 8 - math.floor(b.y / 10) + mi * 2, true)
      end
    end
    b.vx, b.vy = b.vx * 0.985, b.vy * 0.985
    b.vx, b.vy = util.clamp(b.vx, -4, 4), util.clamp(b.vy, -4, 4)
    b.x, b.y = b.x + b.vx, b.y + b.vy
    if b.x < 2 or b.x > 125 then b.vx = -b.vx * 0.8 b.x = util.clamp(b.x, 2, 125) end
    if b.y < 12 or b.y > 54 then b.vy = -b.vy * 0.8 b.y = util.clamp(b.y, 12, 54) end
  end
  for i = 1, #bits do
    for j = i + 1, #bits do
      local a, b = bits[i], bits[j]
      local dx, dy = b.x - a.x, b.y - a.y
      if dx * dx + dy * dy < 9 and a.cool == 0 and b.cool == 0 then
        a.vx, b.vx = b.vx, a.vx
        a.vy, b.vy = b.vy, a.vy
        a.cool, b.cool = 10, 10
        ring((a.x + b.x) / 2, (a.y + b.y) / 2, 1 + math.floor((54 - a.y) / 6), false)
      end
    end
  end
end

function enc(n, d)
  if n == 2 then mags[1].x = util.clamp(mags[1].x + d * 2, 8, 120)
  elseif n == 3 then mags[2].x = util.clamp(mags[2].x + d * 2, 8, 120) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then mags[2].pol = -mags[2].pol kick(2)
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  for _, m in ipairs(mags) do
    screen.level(m.pol > 0 and 15 or 6)
    screen.rect(m.x - 3, m.y - 3, 6, 6)
    screen.fill()
    screen.level(0)
    screen.move(m.x - 2, m.y + 2)
    screen.text(m.pol > 0 and "+" or "-")
  end
  for _, r in ipairs(rings) do
    screen.level(math.max(1, 10 - r.r))
    screen.circle(r.x, r.y, r.r)
    screen.stroke()
  end
  screen.level(12)
  for _, b in ipairs(bits) do screen.rect(b.x - 1, b.y - 1, 2, 2) screen.fill() end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "magnets (paused)" or "magnets")
  screen.level(4)
  screen.move(0, 62)
  screen.text("pull " .. params:string("strength"))
  screen.move(127, 62)
  screen.text_right(mags[2].pol > 0 and "both pull" or "pull / push")
  screen.update()
end
