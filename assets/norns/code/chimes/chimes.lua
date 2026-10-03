-- chimes
-- a Portamax norns script
--
-- wind chimes on a porch. the wind
-- rises and falls on its own; the
-- stronger it blows, the more the
-- tubes knock together. they're
-- tuned to a just-intonation
-- pentatonic set, not equal
-- temperament.
--
-- E2 wind   E3 tubes length
-- K2 gust   K3 calm
-- (params: fundamental)

engine.name = 'PolyPerc'

-- 1/1 9/8 5/4 3/2 5/3 2/1 9/4 5/2: pure intervals, the way chimes
-- are cut to length rather than tuned to a piano
local RATIOS = { 1, 9 / 8, 5 / 4, 3 / 2, 5 / 3, 2, 9 / 4, 5 / 2 }
local swing = {}
local wind = 0.5
local t = 0

function init()
  params:add_separator("CHIMES")
  params:add_control("fund", "fundamental", controlspec.new(110, 440, 'exp', 0, 262, 'hz'))
  params:add_control("wind", "wind", controlspec.new(0, 1, 'lin', 0, 0.5, ''))
  params:add_control("ring", "ring", controlspec.new(1, 8, 'exp', 0, 4, 's'))
  params:set_action("ring", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(5000)
  engine.pw(0.05)
  engine.amp(0.15)
  for i = 1, #RATIOS do swing[i] = 0 end
  math.randomseed(os.time())
  local m = metro.init(function()
    t = t + 1 / 30
    -- the wind wanders around its setting
    local target = params:get("wind") * (0.6 + 0.4 * math.sin(t * 0.3) + 0.2 * math.sin(t * 1.7))
    wind = wind + (target - wind) * 0.02
    for i = 1, #RATIOS do
      swing[i] = swing[i] * 0.9 + (math.random() - 0.5) * wind * 2
      if math.abs(swing[i]) > 1 and math.random() < wind then
        engine.pan((i - 4.5) / 4)
        engine.amp(0.05 + math.min(0.2, math.abs(swing[i]) * 0.08))
        engine.hz(params:get("fund") * RATIOS[i])
        swing[i] = -swing[i] * 0.5
      end
    end
    redraw()
  end, 1 / 30)
  m:start()
end

function enc(n, d)
  if n == 2 then params:delta("wind", d)
  elseif n == 3 then params:delta("ring", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then wind = 1 elseif n == 3 then wind = 0 end
end

function redraw()
  screen.clear()
  screen.level(6)
  screen.move(14, 12)
  screen.line(114, 12)
  screen.stroke()
  for i = 1, #RATIOS do
    local x = 14 + (i - 1) * 14 + swing[i] * 4
    local len = 44 / RATIOS[i] ^ 0.5
    screen.level(math.abs(swing[i]) > 0.8 and 15 or 5)
    screen.move(14 + (i - 1) * 14, 12)
    screen.line(x, 12 + len)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("chimes")
  screen.level(4)
  screen.move(127, 62)
  screen.text_right(string.format("wind %.0f%%", wind * 100))
  screen.update()
end
