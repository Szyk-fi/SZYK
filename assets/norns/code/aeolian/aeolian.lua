-- aeolian
-- a Portamax norns script
--
-- a wind harp: one long string in
-- the wind. eddies shed off the
-- string at a rate set by the wind
-- speed, and the string answers
-- with whichever natural harmonic
-- lies nearest -- so a rising wind
-- climbs the overtone series.
--
-- E2 wind   E3 gustiness
-- K2 gust   K3 still air
-- pads: pluck a harmonic
-- (params: fundamental, string)

engine.name = 'PolyPerc'

local wind = 0
local gust = 0
local still = false
local lit = 1
local glow = 0
local streaks = {}
local t = 0

-- the harmonic the wind is "tuned" to right now (Strouhal: f = 0.2 U / d)
local function centre()
  return util.clamp(0.2 * wind / params:get("string") / params:get("f0") * 1000, 1, 16)
end

local function sound(h)
  lit, glow = h, 15
  local f = params:get("f0") * h
  engine.pan(math.sin(t * 0.7) * 0.5)
  engine.pw(0.5 - math.min(h, 12) * 0.025)
  engine.amp(util.clamp(0.1 + wind / 60, 0.1, 0.32))
  engine.release(2 + 2 / h)
  engine.hz(f)
end

function init()
  params:add_separator("AEOLIAN")
  params:add_control("f0", "fundamental", controlspec.new(40, 130, 'exp', 0, 65.4, 'hz'))
  params:add_control("string", "string", controlspec.new(0.5, 3, 'lin', 0.1, 1.4, 'mm'))
  params:add_control("wind", "wind", controlspec.new(0, 15, 'lin', 0.1, 5, 'm/s'))
  params:add_control("gusty", "gustiness", controlspec.new(0, 1, 'lin', 0.01, 0.4, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 1600, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  wind = params:get("wind")
  sound(math.floor(centre() + 0.5))
  for i = 1, 16 do streaks[i] = { x = math.random(0, 127), y = math.random(12, 54) } end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then sound((msg.note % 12) + 2) end
  end
  clock.run(function()
    while true do
      clock.sleep(0.1)
      t = t + 0.1
      -- gusts: a mean-reverting random wander on top of the set wind
      local g = params:get("gusty")
      gust = gust * 0.97 + (math.random() - 0.5) * g * 1.2
      local target = still and 0 or params:get("wind") + gust
      wind = math.max(0, wind + (target - wind) * 0.15)
      if not still and math.random() < 0.08 + wind / 25 then
        local c = centre()
        -- lock in to the nearest harmonic, sometimes its neighbour
        local h = math.floor(c + 0.5) + (math.random() < 0.3 and math.random(-1, 1) or 0)
        sound(util.clamp(h, 1, 16))
      end
    end
  end)
  metro.init(function()
    glow = math.max(0, glow - 0.5)
    for _, s in ipairs(streaks) do
      s.x = s.x + 0.5 + wind * 0.5
      if s.x > 130 then s.x, s.y = -8, math.random(12, 54) end
    end
    redraw()
  end, 1 / 30):start()
end

function enc(n, d)
  if n == 2 then params:delta("wind", d)
  elseif n == 3 then params:delta("gusty", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then gust = gust + 5 sound(util.clamp(math.floor(centre() + 2.5), 1, 16))
  elseif n == 3 then still = not still end
end

function redraw()
  screen.clear()
  screen.level(2)
  for _, s in ipairs(streaks) do
    screen.move(s.x, s.y)
    screen.line(s.x + 3 + wind * 0.6, s.y)
    screen.stroke()
  end
  -- the string, drawn in the shape of the harmonic it is singing
  local amp = glow * 0.5
  screen.level(math.floor(4 + glow * 0.7))
  screen.move(8, 33)
  for x = 8, 120, 2 do
    local u = (x - 8) / 112
    screen.line(x, 33 + math.sin(u * lit * math.pi) * amp * math.cos(t * 9 + x))
  end
  screen.stroke()
  screen.level(15)
  screen.rect(6, 29, 2, 8)
  screen.rect(120, 29, 2, 8)
  screen.fill()
  screen.move(0, 8)
  screen.text(still and "aeolian (still)" or "aeolian")
  screen.move(127, 8)
  screen.text_right("harmonic " .. lit)
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.format("wind %.1f m/s", wind))
  screen.move(127, 62)
  screen.text_right(string.format("%.0f hz", params:get("f0") * lit))
  screen.update()
end
