-- tempo
-- a Portamax norns script
--
-- a metronome with a feel. pick a
-- meter and its accent grouping,
-- add subdivisions under each beat,
-- and tap K3 in time to set the
-- tempo (two taps or more).
--
-- E2 meter   E3 subdivision
-- K2 restart bar   K3 tap tempo
-- E1 tempo
-- (params: click tone, sub level)

engine.name = 'PolyPerc'

local METERS = {
  { name = "4/4", acc = { 3, 1, 2, 1 } },
  { name = "3/4", acc = { 3, 1, 1 } },
  { name = "6/8", acc = { 3, 1, 1, 2, 1, 1 } },
  { name = "5/4 3+2", acc = { 3, 1, 1, 2, 1 } },
  { name = "7/8 2+2+3", acc = { 3, 1, 2, 1, 2, 1, 1 } },
}
local SUBS = { "none", "8ths", "triplets", "16ths" }
local SUB_N = { 1, 2, 3, 4 }
local beat = 0
local subi = 0
local taps = {}
local flash = 0

local function click(level)
  local freqs = { 1320, 1760, 2640 }
  if level == 0 then
    engine.amp(0.12 * params:get("sublevel"))
    engine.cutoff(params:get("tone") * 0.6)
    engine.release(0.03)
    engine.hz(990)
  else
    engine.amp(({ 0.18, 0.24, 0.32 })[level])
    engine.cutoff(params:get("tone"))
    engine.release(level == 3 and 0.08 or 0.05)
    engine.hz(freqs[level])
  end
end

local function tap()
  local now = util.time()
  -- a long pause starts a fresh count
  if #taps > 0 and now - taps[#taps] > 2 then taps = {} end
  taps[#taps + 1] = now
  while #taps > 5 do table.remove(taps, 1) end
  if #taps >= 2 then
    local span = (taps[#taps] - taps[1]) / (#taps - 1)
    if span >= 0.25 then
      params:set("clock_tempo", util.clamp(util.round(60 / span), 40, 240))
    end
  end
end

function init()
  local names = {}
  for i, m in ipairs(METERS) do names[i] = m.name end
  params:add_separator("TEMPO")
  params:add_option("meter", "meter", names, 1)
  params:add_option("sub", "subdivision", SUBS, 1)
  params:add_control("tone", "click tone", controlspec.new(1000, 8000, 'exp', 0, 4000, 'hz'))
  params:add_control("sublevel", "sub level", controlspec.new(0, 1, 'lin', 0, 0.6, ''))
  params:default()
  engine.pw(0.5)
  engine.gain(1)
  clock.run(function()
    while true do
      local n = SUB_N[params:get("sub")]
      clock.sync(1 / n)
      subi = subi % n + 1
      if subi == 1 then
        local acc = METERS[params:get("meter")].acc
        beat = beat % #acc + 1
        click(acc[beat])
        flash = 15
      else
        click(0)
      end
      redraw()
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      flash = math.max(0, flash - 2)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then params:delta("meter", d) beat = 0
  elseif n == 3 then params:delta("sub", d) subi = 0 end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then beat = 0 subi = 0
  elseif n == 3 then tap() end
  redraw()
end

function redraw()
  screen.clear()
  local acc = METERS[params:get("meter")].acc
  local w = 120 / #acc
  for i, a in ipairs(acc) do
    local h = a * 8
    screen.level(i == beat and math.max(6, flash) or 2 + a)
    screen.rect(4 + (i - 1) * w, 48 - h, w - 3, h)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("tempo")
  screen.move(127, 8)
  screen.text_right(util.round(clock.get_tempo()) .. " bpm")
  screen.level(4)
  screen.move(0, 62)
  screen.text(METERS[params:get("meter")].name)
  screen.move(127, 62)
  screen.text_right(SUBS[params:get("sub")])
  screen.update()
end
