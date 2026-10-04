-- rainfall
-- a Portamax norns script
--
-- rain falls on a tin roof of
-- tuned panels. wind blows the
-- drops sideways, across the
-- roof and across the stereo field.
--
-- E2 rain   E3 wind
-- K2 gust   K3 thunder
-- (params: root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local drops = {}
local splashes = {}
local wind = 0
local gust = 0
local panels = {}
local rumble = 0

local function tune()
  panels = MusicUtil.generate_scale_of_length(params:get("root"), "major pentatonic", 8)
end

function init()
  params:add_separator("RAINFALL")
  params:add_number("root", "root", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", tune)
  params:add_number("rain", "rain", 0, 20, 6)
  params:add_number("wind", "wind", -10, 10, 0)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.5, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(3200)
  engine.amp(0.2)
  engine.pw(0.1)
  tune()
  math.randomseed(os.time())
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      step()
      redraw()
    end
  end)
end

function step()
  wind = params:get("wind") / 10 + gust
  gust = gust * 0.97
  if math.random() < params:get("rain") / 40 then
    table.insert(drops, { x = math.random(0, 127), y = 0, v = 1.5 + math.random() })
  end
  for i = #drops, 1, -1 do
    local d = drops[i]
    d.y = d.y + d.v
    d.x = (d.x + wind * 1.5) % 128
    if d.y >= 50 then
      local p = math.floor(d.x / 16) + 1
      engine.pan(d.x / 64 - 1)
      engine.amp(0.1 + d.v * 0.05)
      engine.hz(MusicUtil.note_num_to_freq(panels[p]))
      table.insert(splashes, { x = d.x, r = 1 })
      table.remove(drops, i)
    end
  end
  for i = #splashes, 1, -1 do
    splashes[i].r = splashes[i].r + 1
    if splashes[i].r > 6 then table.remove(splashes, i) end
  end
  if rumble > 0 then
    rumble = rumble - 1
    if rumble % 6 == 0 then
      engine.pan(0)
      engine.amp(0.3)
      engine.hz(MusicUtil.note_num_to_freq(params:get("root") - 24))
    end
  end
end

function enc(n, d)
  if n == 2 then params:delta("rain", d)
  elseif n == 3 then params:delta("wind", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then gust = (math.random() < 0.5 and -1 or 1) * 2
  elseif n == 3 then rumble = 36 end
end

function redraw()
  screen.clear()
  screen.level(rumble > 30 and 15 or 0)
  if rumble > 30 then screen.rect(0, 0, 128, 64) screen.fill() end
  for p = 0, 7 do
    screen.level(3 + p % 2 * 2)
    screen.rect(p * 16 + 1, 51, 14, 3)
    screen.fill()
  end
  screen.level(10)
  for _, d in ipairs(drops) do
    screen.move(d.x, d.y)
    screen.line(d.x - wind * 2, d.y - 3)
    screen.stroke()
  end
  for _, s in ipairs(splashes) do
    screen.level(math.max(1, 12 - s.r * 2))
    screen.circle(s.x, 50, s.r)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("rainfall")
  screen.level(4)
  screen.move(0, 62)
  screen.text("rain " .. params:get("rain"))
  screen.move(127, 62)
  screen.text_right("wind " .. params:get("wind"))
  screen.update()
end
