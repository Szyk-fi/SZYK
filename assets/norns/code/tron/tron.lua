-- tron
-- a Portamax norns script
--
-- two light-cycles race across
-- a grid, walls of light behind
-- them. every turn is a note;
-- the one who crashes ends the
-- round with a chord.
--
-- E2 speed       E3 boldness
-- K2 new round   K3 swap scales
-- pads: nudge a cycle
-- (params: key, scales)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H = 63, 27       -- cells (2 px each), field from y = 9
local grid_ = {}
local bikes = {}
local DIRS = { { 1, 0 }, { 0, 1 }, { -1, 0 }, { 0, -1 } }
local state = "race"
local wait = 0
local wins = { 0, 0 }
local steps = 0
local flash = 0
local scales = {}
local swapped = false
local SCALE_NAMES = { "Dorian", "Phrygian", "Lydian", "Mixolydian", "Minor Pentatonic", "Whole Tone" }

local function build_scales()
  local root = params:get("key")
  local a, b = SCALE_NAMES[params:get("scale1")], SCALE_NAMES[params:get("scale2")]
  if swapped then a, b = b, a end
  scales[1] = MusicUtil.generate_scale_of_length(root, a, 14)
  scales[2] = MusicUtil.generate_scale_of_length(root + 7, b, 14)
end

local function cell(x, y)
  if x < 1 or x > W or y < 1 or y > H then return 99 end
  return grid_[y][x]
end

local function new_round()
  for y = 1, H do
    grid_[y] = {}
    for x = 1, W do grid_[y][x] = 0 end
  end
  bikes = {
    { x = 10, y = 7, d = 1, alive = true, turns = 0 },
    { x = W - 9, y = 21, d = 3, alive = true, turns = 0 },
  }
  for i, b in ipairs(bikes) do grid_[b.y][b.x] = i end
  state = "race"
  steps = 0
end

local function tone(n, amp, rel, cut, pan, pw)
  engine.pan(pan)
  engine.pw(pw or 0.5)
  engine.cutoff(cut)
  engine.release(rel)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

-- free cells reachable from (x, y), counted up to a limit
local function room(x, y, limit)
  if cell(x, y) ~= 0 then return 0 end
  local seen = { [y * 100 + x] = true }
  local q = { { x, y } }
  local n, h = 0, 1
  while h <= #q and n < limit do
    local p = q[h]
    h = h + 1
    n = n + 1
    for _, d in ipairs(DIRS) do
      local nx, ny = p[1] + d[1], p[2] + d[2]
      local k = ny * 100 + nx
      if not seen[k] and cell(nx, ny) == 0 then
        seen[k] = true
        q[#q + 1] = { nx, ny }
      end
    end
  end
  return n
end

local function choose(b)
  local opts = { b.d, (b.d % 4) + 1, ((b.d + 2) % 4) + 1 } -- straight, right, left
  local best, bs = b.d, -1
  for i, d in ipairs(opts) do
    local nx, ny = b.x + DIRS[d][1], b.y + DIRS[d][2]
    local s = room(nx, ny, 60)
    -- like going straight, but now and then take a bold turn
    if i == 1 then s = s + 4 end
    if i > 1 and math.random() < params:get("bold") * 0.15 then s = s + 10 end
    if s > bs then best, bs = d, s end
  end
  return best
end

local function turn_note(i, b)
  b.turns = b.turns + 1
  -- the pitch follows where on the grid the turn happens
  local deg = util.clamp(math.floor(util.linlin(1, H, 14, 1, b.y)), 1, 14)
  tone(scales[i][deg], 0.26, 0.4, 2600, i == 1 and -0.6 or 0.6, i == 1 and 0.3 or 0.6)
  if b.turns % 4 == 0 then
    tone(scales[i][1] - 12, 0.18, 0.6, 600, i == 1 and -0.3 or 0.3, 0.5)
  end
end

local function crash()
  state = "over"
  wait = 1.6
  flash = 15
  local a, b = bikes[1].alive, bikes[2].alive
  local root = params:get("key") - 12
  local chord
  if a and not b then wins[1] = wins[1] + 1 chord = { 0, 4, 7, 11, 14 }
  elseif b and not a then wins[2] = wins[2] + 1 chord = { 0, 3, 7, 10, 14 }
  else chord = { 0, 5, 10, 15, 20 } end -- head-on: a stack of fourths
  for k, iv in ipairs(chord) do tone(root + iv, 0.18, 2.2, 1800, util.linlin(1, #chord, -0.5, 0.5, k), 0.4) end
end

local function step()
  if state == "over" then return end
  steps = steps + 1
  local moves = {}
  for i, b in ipairs(bikes) do
    local d = choose(b)
    if d ~= b.d then
      b.d = d
      turn_note(i, b)
    end
    moves[i] = { b.x + DIRS[b.d][1], b.y + DIRS[b.d][2] }
  end
  for i, b in ipairs(bikes) do
    local nx, ny = moves[i][1], moves[i][2]
    local other = moves[3 - i]
    if cell(nx, ny) ~= 0 or (nx == other[1] and ny == other[2]) then
      b.alive = false
    end
  end
  for i, b in ipairs(bikes) do
    if b.alive then
      b.x, b.y = moves[i][1], moves[i][2]
      grid_[b.y][b.x] = i
    end
  end
  if not (bikes[1].alive and bikes[2].alive) then crash() end
  -- a soft pulse every eight cells keeps time
  if steps % 8 == 0 and state == "race" then
    tone(params:get("key") - 24, 0.2, 0.25, 400, 0, 0.5)
  end
end

function init()
  params:add_separator("TRON")
  params:add_number("key", "key", 48, 64, 52, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_scales)
  params:add_option("scale1", "cycle 1 scale", SCALE_NAMES, 1)
  params:set_action("scale1", build_scales)
  params:add_option("scale2", "cycle 2 scale", SCALE_NAMES, 5)
  params:set_action("scale2", build_scales)
  params:add_number("speed", "cells/s", 5, 40, 16)
  params:add_control("bold", "boldness", controlspec.new(0, 1, 'lin', 0, 0.4, ''))
  params:default()
  engine.gain(1.4)
  math.randomseed(os.time())
  build_scales()
  new_round()
  -- the start: both engines rev
  tone(scales[1][1], 0.25, 0.5, 2000, -0.5, 0.3)
  tone(scales[2][1], 0.25, 0.5, 2000, 0.5, 0.6)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" and state == "race" then
      local i = (msg.note % 2) + 1
      local b = bikes[i]
      local d = (b.d % 4) + 1
      local nx, ny = b.x + DIRS[d][1], b.y + DIRS[d][2]
      if cell(nx, ny) == 0 then
        b.d = d
        turn_note(i, b)
      end
    end
  end
  clock.run(function()
    while true do
      if state == "over" then
        clock.sleep(wait)
        new_round()
        tone(scales[1][1], 0.22, 0.4, 2000, -0.5, 0.3)
        tone(scales[2][1], 0.22, 0.4, 2000, 0.5, 0.6)
      end
      step()
      clock.sleep(1 / params:get("speed"))
    end
  end)
  local frame = metro.init(function()
    flash = math.max(0, flash - 1)
    redraw()
  end, 1 / 30)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bold", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    new_round()
    tone(scales[1][1], 0.22, 0.4, 2000, -0.5, 0.3)
  elseif n == 3 then
    swapped = not swapped
    build_scales()
    tone(scales[1][3], 0.2, 0.3, 2400, 0, 0.5)
  end
  redraw()
end

function redraw()
  screen.clear()
  -- faint grid lines
  screen.level(1)
  for x = 0, W * 2, 16 do
    screen.move(x + 1, 9)
    screen.line(x + 1, 9 + H * 2)
    screen.stroke()
  end
  -- trails, one fill per cycle
  for who = 1, 2 do
    screen.level(who == 1 and 10 or 5)
    for y = 1, H do
      local row = grid_[y]
      for x = 1, W do
        if row[x] == who then screen.rect(x * 2 - 1, 7 + y * 2, 2, 2) end
      end
    end
    screen.fill()
  end
  for i, b in ipairs(bikes) do
    screen.level(b.alive and 15 or (flash > 0 and flash or 2))
    screen.rect(b.x * 2 - 2, 6 + b.y * 2, 4, 4)
    if b.alive then screen.fill() else screen.stroke() end
  end
  screen.level(1)
  screen.rect(0, 8, 128, H * 2 + 2)
  screen.stroke()
  screen.level(10)
  screen.move(0, 6)
  screen.text(wins[1])
  screen.level(5)
  screen.move(127, 6)
  screen.text_right(wins[2])
  screen.level(3)
  screen.move(64, 6)
  screen.text_center(state == "over" and "derezzed" or "tron")
  screen.update()
end
