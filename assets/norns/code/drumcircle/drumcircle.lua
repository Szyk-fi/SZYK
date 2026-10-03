-- drumcircle
-- a Portamax norns script
--
-- four hand drums sit in a circle,
-- each spreading its hits evenly
-- (euclidean) over sixteen steps.
-- tune them low, keep them short,
-- and let the rhythms interlock.
--
-- E2 pick drum   E3 hits
-- K2 shuffle all   K3 pause
-- (params: hits, rotation, swing)

engine.name = 'PolyPerc'

local DRUMS = {
  { name = "kick", hz = 52, cut = 260, rel = 0.25, amp = 0.35, pan = 0 },
  { name = "tom", hz = 98, cut = 520, rel = 0.18, amp = 0.3, pan = -0.4 },
  { name = "rim", hz = 330, cut = 1500, rel = 0.05, amp = 0.22, pan = 0.5 },
  { name = "shkr", hz = 1900, cut = 3200, rel = 0.03, amp = 0.15, pan = 0.2 },
}
local DEFAULT_HITS = { 4, 3, 5, 7 }
local STEPS = 16
local step = 0
local sel = 1
local paused = false
local flash = { 0, 0, 0, 0 }

local function hit_at(d, s)
  local k = params:get("hits" .. d)
  local r = params:get("rot" .. d)
  local i = (s - 1 + r) % STEPS
  return k > 0 and ((i * k) % STEPS) < k
end

local function play(d, accent)
  local dr = DRUMS[d]
  engine.cutoff(dr.cut * (accent and 1.4 or 1))
  engine.release(dr.rel)
  engine.amp(dr.amp * (accent and 1.2 or 0.85))
  engine.pan(dr.pan)
  engine.pw(0.5)
  engine.hz(dr.hz * (1 + (math.random() - 0.5) * 0.02))
  flash[d] = 15
end

function init()
  params:add_separator("DRUMCIRCLE")
  for d = 1, 4 do
    params:add_number("hits" .. d, DRUMS[d].name .. " hits", 0, STEPS, DEFAULT_HITS[d])
    params:add_number("rot" .. d, DRUMS[d].name .. " rotation", 0, STEPS - 1, (d - 1) * 2)
  end
  params:add_control("swing", "swing", controlspec.new(0, 0.3, 'lin', 0, 0.08, ''))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  clock.run(function()
    while true do
      local sw = (step % 2 == 1) and params:get("swing") or 0
      clock.sync(1 / 4, sw / 4)
      step = step % STEPS + 1
      if not paused then
        for d = 1, 4 do
          if hit_at(d, step) then play(d, step % 4 == 1) end
        end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then sel = util.clamp(sel + d, 1, 4)
  elseif n == 3 then params:delta("hits" .. sel, d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    for d = 1, 4 do
      params:set("hits" .. d, math.random(d == 4 and 4 or 2, d == 4 and 11 or 7))
      params:set("rot" .. d, math.random(0, STEPS - 1))
    end
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local cx, cy = 64, 34
  for d = 1, 4 do
    local r = 8 + d * 5
    for s = 1, STEPS do
      local a = (s - 1) / STEPS * 2 * math.pi - math.pi / 2
      local x, y = cx + math.cos(a) * r, cy + math.sin(a) * r * 0.85
      if hit_at(d, s) then
        screen.level(s == step and 15 or (d == sel and 9 or 4))
        screen.rect(x - 1, y - 1, 2, 2)
      else
        screen.level(s == step and 6 or 1)
        screen.pixel(x, y)
      end
      screen.fill()
    end
  end
  for d = 1, 4 do
    screen.level(math.max(2, flash[d]))
    screen.circle(cx, cy, 1 + d)
    screen.stroke()
    flash[d] = math.max(0, flash[d] - 3)
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "drumcircle (paused)" or "drumcircle")
  screen.level(4)
  screen.move(0, 62)
  screen.text(DRUMS[sel].name)
  screen.move(127, 62)
  screen.text_right(params:get("hits" .. sel) .. "/" .. STEPS)
  screen.update()
end
