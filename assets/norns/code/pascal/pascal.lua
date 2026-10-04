-- pascal
-- a Portamax norns script
--
-- pascal's triangle, each entry
-- the sum of the two above it,
-- kept modulo a small prime. with
-- mod 2 it draws sierpinski's
-- triangle. each row is a rhythm:
-- non-zero entries play, zeros
-- rest, and the outer edges ring
-- low while the middle rings high.
--
-- E2 row   E3 modulus
-- K2 random row   K3 pause
-- pads: root note
-- (params: repeats, scale)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local ROWS = 32
local MODS = { 2, 3, 5, 7 }
local tri = {}
local row = 7
local col = 0
local rep = 1
local lastr, lastk = 0, 0
local root = 50
local scale = {}
local paused = false

local function build()
  local p = MODS[params:get("mod")]
  tri = { [0] = { [0] = 1 } }
  for r = 1, ROWS - 1 do
    tri[r] = {}
    for k = 0, r do
      tri[r][k] = ((tri[r - 1][k - 1] or 0) + (tri[r - 1][k] or 0)) % p
    end
  end
end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 16)
end

local function tick()
  local v = tri[row][col]
  lastr, lastk = row, col
  if v ~= 0 then
    -- symmetric rows rise to the middle and fall back
    local inward = math.min(col, row - col)
    local deg = math.floor(inward / math.max(1, row / 2) * 9) + v % 3 + 1
    engine.amp(col == 0 and 0.3 or 0.2)
    engine.pan(util.linlin(0, row, -0.7, 0.7, col))
    engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
  end
  if col == 0 then
    engine.amp(0.16)
    engine.hz(MusicUtil.note_num_to_freq(root - 12))
  end
  col = col + 1
  if col > row then
    col = 0
    rep = rep + 1
    if rep > params:get("repeats") then
      rep = 1
      row = row % (ROWS - 1) + 1
    end
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("PASCAL")
  params:add_option("mod", "modulus", { "2", "3", "5", "7" }, 1)
  params:set_action("mod", build)
  params:add_number("repeats", "repeats per row", 1, 4, 2)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_control("release", "release", controlspec.new(0.1, 2, 'exp', 0, 0.5, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2000)
  engine.pw(0.35)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 10 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then tick() end
      redraw()
      clock.sync(1 / 4)
    end
  end)
end

function enc(n, d)
  if n == 2 then row = util.clamp(row + d, 1, ROWS - 1) col = 0 rep = 1
  elseif n == 3 then params:delta("mod", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then row = math.random(3, ROWS - 1) col = 0 rep = 1
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  for r = 0, ROWS - 1 do
    local y = 12 + r * 1.35
    for k = 0, r do
      local v = tri[r][k]
      if v ~= 0 then
        local here = r == lastr and k == lastk
        screen.level(here and 15 or (r == lastr and 9 or 2 + v))
        screen.rect(64 + (2 * k - r) * 1.9, y, here and 2 or 1, here and 2 or 1)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("pascal")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right("mod " .. MODS[params:get("mod")])
  screen.move(0, 62)
  screen.text("row " .. row)
  screen.move(127, 62)
  screen.text_right(paused and "paused" or (rep .. "/" .. params:get("repeats")))
  screen.update()
end
