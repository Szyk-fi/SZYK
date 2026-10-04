-- markov
-- a Portamax norns script
--
-- teach it a melody with the pads
-- (or any MIDI keyboard). it learns
-- which note tends to follow which,
-- then improvises in that habit.
--
-- E2 tempo   E3 wander
-- K2 forget   K3 play / listen
-- (params: octave shift)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local follows = {}     -- follows[a][b] = how often b came after a
local last_heard = nil
local current = nil
local playing = true
local history = {}

local function learn(note)
  if last_heard then
    follows[last_heard] = follows[last_heard] or {}
    follows[last_heard][note] = (follows[last_heard][note] or 0) + 1
  end
  last_heard = note
end

local function choose(from)
  local options = follows[from]
  if options == nil or math.random() < params:get("wander") then
    -- wander: jump to any note it has heard
    local all = {}
    for a, _ in pairs(follows) do all[#all + 1] = a end
    if #all == 0 then return nil end
    return all[math.random(#all)]
  end
  local total = 0
  for _, w in pairs(options) do total = total + w end
  local r = math.random() * total
  for b, w in pairs(options) do
    r = r - w
    if r <= 0 then return b end
  end
  return nil
end

local function play(note)
  engine.hz(MusicUtil.note_num_to_freq(note + params:get("shift") * 12))
  table.insert(history, note)
  while #history > 24 do table.remove(history, 1) end
end

function init()
  params:add_separator("MARKOV")
  params:add_number("shift", "octave shift", -2, 2, 0)
  params:add_control("wander", "wander", controlspec.new(0, 1, 'lin', 0, 0.1, ''))
  params:default()
  engine.release(0.8)
  engine.cutoff(2200)
  engine.amp(0.3)
  -- a little seed melody so it has something to say at first
  for _, n in ipairs({ 60, 62, 64, 67, 64, 62, 60, 67, 69, 67, 64, 62, 60 }) do learn(n) end
  last_heard = nil
  current = 60
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      learn(msg.note)
      play(msg.note)
      current = msg.note
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      if playing and current then
        local nxt = choose(current)
        if nxt then current = nxt play(nxt) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("clock_tempo", d)
  elseif n == 3 then params:delta("wander", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then follows = {} history = {} last_heard = nil
  elseif n == 3 then playing = not playing end
  redraw()
end

function redraw()
  screen.clear()
  local known = 0
  for _ in pairs(follows) do known = known + 1 end
  for i, n in ipairs(history) do
    screen.level(i == #history and 15 or 3 + i // 3)
    screen.rect(4 + (i - 1) * 5, 52 - (n - 48) * 1.3, 4, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text(playing and "markov" or "markov (listening)")
  screen.level(4)
  screen.move(0, 62)
  screen.text(known .. " notes known")
  screen.move(127, 62)
  screen.text_right("wander " .. params:string("wander"))
  screen.update()
end
