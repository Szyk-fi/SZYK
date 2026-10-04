-- clapping
-- a Portamax norns script
--
-- one twelve-step cell, two
-- players. the first holds
-- still; the second slips one
-- step ahead every few bars
-- until they meet again.
--
-- E2 tempo     E3 bars per shift
-- K2 new cell  K3 shift now
-- pads: clap along
-- (params: pitches, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local STEPS = 12
local cell = {}
local step = 0
local bar = 0
local offset = 0
local flash = { 0, 0 }
local pad_flash = 0

local function new_cell()
  -- a cell of 7 or 8 hits that always starts on the downbeat and
  -- never leaves more than two rests in a row
  repeat
    cell = {}
    local hits = 0
    for i = 1, STEPS do
      cell[i] = (i == 1 or math.random() < 0.62) and 1 or 0
      hits = hits + cell[i]
    end
    local ok = hits >= 7 and hits <= 8
    for i = 1, STEPS do
      if cell[i] == 0 and cell[i % STEPS + 1] == 0 and cell[(i + 1) % STEPS + 1] == 0 then ok = false end
    end
  until ok
end

local function clap(voice)
  local base = params:get("pitch" .. voice)
  -- a clap: a burst of two very short pulses close together
  engine.pan(voice == 1 and -0.6 or 0.6)
  engine.release(params:get("snap"))
  engine.pw(0.08)
  engine.cutoff(params:get("tone"))
  engine.amp(0.35)
  engine.hz(MusicUtil.note_num_to_freq(base))
  engine.hz(MusicUtil.note_num_to_freq(base + 7) * 1.013)
  flash[voice] = 15
end

local function tick()
  local i = step % STEPS + 1
  if cell[i] == 1 then clap(1) end
  local j = (step + offset) % STEPS + 1
  if cell[j] == 1 then clap(2) end
  -- a quiet pulse on the downbeat keeps the bar audible
  if i == 1 then
    engine.pan(0)
    engine.pw(0.5)
    engine.release(0.25)
    engine.cutoff(500)
    engine.amp(0.18)
    engine.hz(MusicUtil.note_num_to_freq(params:get("pitch1") - 24))
  end
  step = step + 1
  if step % STEPS == 0 then
    bar = bar + 1
    if bar % params:get("bars") == 0 then offset = (offset + 1) % STEPS end
  end
end

function init()
  params:add_separator("CLAPPING")
  params:add_number("bpm", "tempo", 60, 240, 168)
  params:add_number("bars", "bars per shift", 1, 16, 4)
  params:add_number("pitch1", "player 1 pitch", 48, 84, 67, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_number("pitch2", "player 2 pitch", 48, 84, 72, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_control("tone", "tone", controlspec.new(400, 8000, 'exp', 0, 3200, 'hz'))
  params:add_control("snap", "snap", controlspec.new(0.02, 0.4, 'exp', 0, 0.07, 's'))
  params:default()
  engine.gain(2.2)
  math.randomseed(os.time())
  new_cell()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.pan(0)
      engine.pw(0.1)
      engine.release(0.12)
      engine.cutoff(params:get("tone"))
      engine.amp(0.3)
      engine.hz(MusicUtil.note_num_to_freq(msg.note + 12))
      pad_flash = 15
    end
  end
  clock.run(function()
    while true do
      tick()
      -- steps are eighth notes
      clock.sleep(30 / params:get("bpm"))
    end
  end)
  local frame = metro.init(function()
    for v = 1, 2 do flash[v] = math.max(0, flash[v] - 2) end
    pad_flash = math.max(0, pad_flash - 2)
    redraw()
  end, 1 / 30)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("bpm", d)
  elseif n == 3 then params:delta("bars", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_cell()
  elseif n == 3 then offset = (offset + 1) % STEPS end
  redraw()
end

local function ring(cx, cy, r, shift, voice)
  local cur = (step - 1) % STEPS
  for k = 0, STEPS - 1 do
    local idx = (k + shift) % STEPS + 1
    local a = (k / STEPS) * 2 * math.pi - math.pi / 2
    local x, y = cx + math.cos(a) * r, cy + math.sin(a) * r
    local here = (k == cur)
    if cell[idx] == 1 then
      screen.level(here and 15 or (voice == 1 and 7 or 5))
      screen.circle(x, y, here and 3 or 2)
      screen.fill()
    else
      screen.level(here and 8 or 2)
      screen.pixel(x, y)
      screen.fill()
    end
  end
end

function redraw()
  screen.clear()
  local cx, cy = 40, 33
  -- outer ring: player 1, inner ring: player 2 (rotated by its offset)
  screen.level(1)
  screen.circle(cx, cy, 27)
  screen.stroke()
  ring(cx, cy, 27, 0, 1)
  ring(cx, cy, 16, offset, 2)
  -- the hand that points at "now"
  local a = (((step - 1) % STEPS) / STEPS) * 2 * math.pi - math.pi / 2
  screen.level(3)
  screen.move(cx, cy)
  screen.line(cx + math.cos(a) * 10, cy + math.sin(a) * 10)
  screen.stroke()
  screen.level(math.max(2, flash[1]))
  screen.rect(cx - 2, cy - 2, 4, 4)
  screen.fill()
  -- the cell as text, and the phase readout
  screen.level(15)
  screen.move(76, 8)
  screen.text("clapping")
  for v = 1, 2 do
    local y = 20 + (v - 1) * 9
    local s = ""
    for k = 0, STEPS - 1 do
      local idx = (k + (v == 2 and offset or 0)) % STEPS + 1
      s = s .. (cell[idx] == 1 and "x" or ".")
    end
    screen.level(math.max(4, flash[v]))
    screen.move(76, y)
    screen.text(s)
  end
  screen.level(6)
  screen.move(76, 44)
  screen.text("shift " .. offset .. "/12")
  screen.move(76, 53)
  screen.text("bar " .. (bar % params:get("bars")) + 1 .. "/" .. params:get("bars"))
  screen.level(4)
  screen.move(76, 62)
  screen.text(params:get("bpm") .. " bpm")
  if pad_flash > 0 then
    screen.level(pad_flash)
    screen.circle(cx, cy, 31)
    screen.stroke()
  end
  screen.update()
end
