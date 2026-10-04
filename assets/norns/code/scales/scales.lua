-- scales
-- a Portamax norns script
--
-- a scale explorer. every scale in
-- musicutil, played up and back down
-- and drawn twice: as a ring of the
-- twelve pitch classes, and as a
-- staircase of its steps.
--
-- E2 scale   E3 root
-- K2 random scale   K3 auto-advance
-- (params: tone, note length)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local notes = {}
local idx = 0
local dir = 1
local cur = nil
local advance = false

local function build()
  local sc = MusicUtil.SCALES[params:get("scale")]
  notes = MusicUtil.generate_scale(params:get("root"), params:get("scale"), 1)
  idx, dir = 0, 1
  return sc
end

local function next_note()
  idx = idx + dir
  if idx > #notes then dir = -1 idx = #notes - 1 end
  if idx < 1 then
    -- a full run is done: rest a beat, maybe move on
    idx, dir = 0, 1
    if advance then params:set("scale", params:get("scale") % #MusicUtil.SCALES + 1) end
    return nil
  end
  return notes[idx]
end

function init()
  local names = {}
  for i, s in ipairs(MusicUtil.SCALES) do names[i] = string.lower(s.name) end
  params:add_separator("SCALES")
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", build)
  params:add_number("root", "root", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_control("tone", "tone", controlspec.new(500, 6000, 'exp', 0, 2200, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("len", "note length", controlspec.new(0.1, 2, 'lin', 0, 0.5, 's'))
  params:set_action("len", function(x) engine.release(x) end)
  params:default()
  engine.amp(0.25)
  engine.pw(0.45)
  math.randomseed(os.time())
  build()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      cur = next_note()
      if cur then
        engine.pan((idx / #notes - 0.5) * 0.8)
        engine.hz(MusicUtil.note_num_to_freq(cur))
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("scale", d)
  elseif n == 3 then params:delta("root", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then params:set("scale", math.random(#MusicUtil.SCALES))
  elseif n == 3 then advance = not advance end
  redraw()
end

function redraw()
  screen.clear()
  local root = params:get("root")
  local cx, cy, r = 28, 34, 18
  local inscale = {}
  for _, n in ipairs(notes) do inscale[(n - root) % 12] = true end
  for pc = 0, 11 do
    local a = pc / 12 * 2 * math.pi - math.pi / 2
    local x, y = cx + math.cos(a) * r, cy + math.sin(a) * r
    local here = cur and (cur - root) % 12 == pc
    screen.level(here and 15 or (inscale[pc] and 7 or 1))
    screen.circle(x, y, inscale[pc] and 2.5 or 1)
    if inscale[pc] then screen.fill() else screen.stroke() end
  end
  -- staircase: one step per note, height = semitones above root
  local w = 70 / #notes
  for i, n in ipairs(notes) do
    local h = 2 + (n - root) * 2.2
    screen.level(i == idx and 15 or 4)
    screen.rect(54 + (i - 1) * w, 50 - h, math.max(1, w - 1), h)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(advance and "scales >" or "scales")
  screen.move(127, 8)
  screen.text_right(MusicUtil.note_num_to_name(root, true))
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:get("scale") .. " " .. params:string("scale"))
  screen.update()
end
