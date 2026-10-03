-- rings
-- a Portamax norns script
--
-- three euclidean rhythms on
-- three rings: low, middle, high.
-- each ring spreads its hits as
-- evenly as it can over its steps.
--
-- E1 choose ring
-- E2 hits   E3 steps
-- K2 rotate   K3 mute ring
-- (params: tempo, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local rings = {
  { hits = 3, steps = 8, rot = 0, pos = 0, note = 0, muted = false, pattern = {} },
  { hits = 5, steps = 12, rot = 0, pos = 0, note = 7, muted = false, pattern = {} },
  { hits = 7, steps = 16, rot = 0, pos = 0, note = 16, muted = false, pattern = {} },
}
local focus = 1

-- Bjorklund-style spreading: step i is a hit when the running
-- total of hits/steps crosses an integer.
local function euclid(hits, steps, rot)
  local p = {}
  for i = 0, steps - 1 do
    local j = (i + rot) % steps
    p[i + 1] = ((j * hits) % steps) < hits
  end
  return p
end

local function rebuild()
  for _, r in ipairs(rings) do
    r.hits = util.clamp(r.hits, 0, r.steps)
    r.pattern = euclid(r.hits, r.steps, r.rot)
  end
end

function init()
  params:add_separator("RINGS")
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:default()
  engine.amp(0.25)
  engine.release(0.4)
  rebuild()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      for i, r in ipairs(rings) do
        r.pos = r.pos % r.steps + 1
        if r.pattern[r.pos] and not r.muted then
          engine.pan((i - 2) * 0.6)
          engine.cutoff(600 * i * i)
          engine.pw(0.3 + 0.2 * (i - 1))
          engine.hz(MusicUtil.note_num_to_freq(params:get("root") + r.note))
        end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  local r = rings[focus]
  if n == 1 then focus = util.clamp(focus + d, 1, 3)
  elseif n == 2 then r.hits = r.hits + d
  elseif n == 3 then r.steps = util.clamp(r.steps + d, 2, 24) end
  rebuild()
  redraw()
end

function key(n, z)
  if z == 0 then return end
  local r = rings[focus]
  if n == 2 then r.rot = (r.rot + 1) % r.steps rebuild()
  elseif n == 3 then r.muted = not r.muted end
  redraw()
end

function redraw()
  screen.clear()
  local cx, cy = 44, 34
  for i, r in ipairs(rings) do
    local rad = 8 + i * 8
    for s = 1, r.steps do
      local a = (s - 1) / r.steps * 2 * math.pi
      local x = cx + math.sin(a) * rad
      local y = cy - math.cos(a) * rad
      local lvl = r.pattern[s] and 8 or 2
      if s == r.pos then lvl = 15 end
      if r.muted then lvl = math.min(lvl, 3) end
      screen.level(lvl)
      screen.circle(x, y, r.pattern[s] and 2 or 1)
      screen.fill()
    end
  end
  for i, r in ipairs(rings) do
    screen.level(i == focus and 15 or 4)
    screen.move(88, 14 + i * 12)
    screen.text(r.hits .. "/" .. r.steps .. (r.muted and " m" or ""))
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("rings")
  screen.update()
end
