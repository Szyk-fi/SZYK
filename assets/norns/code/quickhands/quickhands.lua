-- quickhands
-- a Portamax norns script
-- after react by dovemouse (monome
-- community): press as many lit keys
-- as you can in ten seconds.
--
-- each hit climbs the scale; a key
-- left alone too long jumps away
-- with a blip. when time is up the
-- grid fills with your score, then a
-- new round starts.
--
-- E2 round length  E3 targets at once
-- K3 start again now
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local targets = {}
local score = 0
local best = 0
local length = 10
local count = 1
local left = 0
local state = "play"
local scale = {}
local round = 0

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

local function note(n, amp)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function new_target(i)
  targets[i] = { x = math.random(1, cols()), y = math.random(1, rows()), age = 0 }
end

local function start()
  round = round + 1
  score = 0
  left = length
  state = "play"
  targets = {}
  for i = 1, count do new_target(i) end
  note(scale[1], 0.2)
end

function grid_redraw()
  g:all(0)
  if state == "play" then
    for _, t in ipairs(targets) do g:led(t.x, t.y, 15 - math.min(t.age, 10)) end
  else
    -- the score, a key per point
    local n = 0
    for y = 1, rows() do
      for x = 1, cols() do
        n = n + 1
        if n <= score then g:led(x, y, n == score and 15 or 6) end
      end
    end
  end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 or state ~= "play" then return end
  for i, t in ipairs(targets) do
    if t.x == x and t.y == y then
      score = score + 1
      note(scale[util.clamp(score, 1, #scale)], 0.4)
      new_target(i)
      break
    end
  end
  grid_redraw()
  redraw()
end

function init()
  scale = MusicUtil.generate_scale_of_length(60, 11, 32)
  engine.release(0.4)
  engine.cutoff(3500)
  start()
  clock.run(function()
    while true do
      clock.sleep(0.1)
      if state == "play" then
        left = left - 0.1
        for i, t in ipairs(targets) do
          t.age = t.age + 1
          -- after a second and a half a target jumps, with a blip
          if t.age > 15 then
            note(scale[1] - 12, 0.15)
            new_target(i)
          end
        end
        if left <= 0 then
          state = "score"
          best = math.max(best, score)
          for k = 1, 3 do note(scale[util.clamp(score, 1, #scale)] + (k - 1) * 4, 0.3) end
          clock.run(function()
            clock.sleep(3)
            start()
          end)
        end
      end
      grid_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then length = util.clamp(length + d, 5, 60)
  elseif n == 3 then count = util.clamp(count + d, 1, 6) end
  redraw()
end

function key(n, z)
  if n == 3 and z == 1 then start() end
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 7)
  screen.text("quickhands")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right("best " .. best)
  screen.level(15)
  screen.move(64, 36)
  screen.text_center(state == "play" and (score .. " hits") or ("score " .. score))
  screen.level(6)
  screen.move(64, 52)
  screen.text_center(state == "play" and string.format("%.1f s left", math.max(left, 0)) or "next round...")
  screen.move(0, 62)
  screen.level(3)
  screen.text(length .. " s rounds, " .. count .. " at once")
  screen.update()
end
