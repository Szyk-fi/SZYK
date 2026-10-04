-- fractree
-- a Portamax norns script
--
-- a tree grows one branch at a
-- time: the trunk, then every fork
-- of the next layer, and so on.
-- each layer sounds an octave
-- higher than the one below; how
-- many right turns a branch took
-- picks its note. then it regrows.
--
-- E2 spread angle   E3 depth
-- K2 new tree   K3 pause
-- pads: root note
-- (params: shrink, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PENTA = { 0, 2, 4, 7, 9, 12, 14 }
local branches = {}
local shown = 0
local root = 36
local paused = false
local jitter = {}

local function grow()
  -- breadth-first, so the tree plays from the trunk upward
  branches = {}
  local depth = params:get("depth")
  local spread = math.rad(params:get("spread"))
  local shrink = params:get("shrink")
  local queue = { { x = 64, y = 56, a = -math.pi / 2, len = 12, d = 1, rights = 0, id = 1 } }
  local head = 1
  while head <= #queue do
    local b = queue[head]
    head = head + 1
    local j = jitter[b.id] or 0
    b.x2 = b.x + math.cos(b.a + j) * b.len
    b.y2 = b.y + math.sin(b.a + j) * b.len
    branches[#branches + 1] = b
    if b.d < depth then
      for side = 0, 1 do
        local turn = side == 0 and -spread or spread
        queue[#queue + 1] = { x = b.x2, y = b.y2, a = b.a + j + turn, len = b.len * shrink,
          d = b.d + 1, rights = b.rights + side, id = b.id * 2 + side }
      end
    end
  end
end

local function new_tree()
  jitter = {}
  for i = 1, 128 do jitter[i] = (math.random() - 0.5) * 0.5 end
  grow()
  shown = 0
end

local function play(b)
  local note = root + 12 * (b.d - 1) + PENTA[b.rights % #PENTA + 1]
  while note > 96 do note = note - 12 end
  engine.amp(util.linlin(1, 6, 0.32, 0.16, b.d))
  engine.release(util.linlin(1, 6, 2.2, 0.4, b.d) * params:get("release"))
  engine.pan(util.linlin(20, 108, -0.8, 0.8, b.x2))
  engine.hz(MusicUtil.note_num_to_freq(note))
end

function init()
  params:add_separator("FRACTREE")
  params:add_number("spread", "spread angle", 5, 80, 28)
  params:set_action("spread", grow)
  params:add_number("depth", "depth", 2, 6, 5)
  params:set_action("depth", function() grow() shown = math.min(shown, #branches) end)
  params:add_control("shrink", "shrink", controlspec.new(0.5, 0.8, 'lin', 0.01, 0.72, ''))
  params:set_action("shrink", grow)
  params:add_control("release", "release scale", controlspec.new(0.3, 2, 'lin', 0, 1, 'x'))
  params:default()
  engine.cutoff(2200)
  engine.pw(0.5)
  new_tree()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 24 end
  end
  clock.run(function()
    while true do
      if not paused then
        shown = shown + 1
        if shown > #branches then
          clock.sync(1)
          new_tree()
          shown = 1
        end
        play(branches[shown])
      end
      redraw()
      clock.sync(1 / 4)
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("spread", d)
  elseif n == 3 then params:delta("depth", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_tree()
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, math.min(shown, #branches) do
    local b = branches[i]
    screen.level(i == shown and 15 or util.clamp(12 - b.d * 2, 3, 10))
    screen.line_width(b.d == 1 and 2 or 1)
    screen.move(b.x, b.y)
    screen.line(b.x2, b.y2)
    screen.stroke()
  end
  screen.line_width(1)
  screen.level(15)
  screen.move(0, 8)
  screen.text("fractree")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:get("spread") .. " deg  depth " .. params:get("depth"))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or (shown .. "/" .. #branches))
  screen.update()
end
