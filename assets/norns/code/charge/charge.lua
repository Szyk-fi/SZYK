-- charge
-- a Portamax norns script
-- after skr by tehn (monome
-- community): charged-up bits
-- release tones.
--
-- hold a key to charge it (it gets
-- brighter). let go and it lets the
-- charge out as pulses, a note each,
-- quieter as it drains. press a
-- pulsing key to silence it.
-- columns are notes, rows how fast
-- a key pulses (top is fastest).
-- now and then a key charges itself.
--
-- E1 tempo  E2 ghost charges  E3 root
-- K2 silence all  K3 charge three
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
-- bits[y][x] = { charge = 0..15, held = bool, t = ticks to next pulse }
local bits = {}
local ghosts = 8
local tick = 0
local root = 52
local scale = {}

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

local function bit(x, y)
  bits[y] = bits[y] or {}
  bits[y][x] = bits[y][x] or { charge = 0, held = false, t = 0 }
  return bits[y][x]
end

local function period(y) return 1 + math.floor((y - 1) * 6 / math.max(rows() - 1, 1)) end

local function pulse(x, y, b)
  local n = scale[util.clamp(x, 1, #scale)]
  engine.amp(0.08 + b.charge / 40)
  engine.hz(MusicUtil.note_num_to_freq(n))
  b.charge = b.charge - 1
  b.t = period(y)
end

local function step()
  tick = tick + 1
  for y, row in pairs(bits) do
    for x, b in pairs(row) do
      if b.held then
        b.charge = math.min(15, b.charge + 1)
      elseif b.charge > 0 then
        b.t = b.t - 1
        if b.t <= 0 then pulse(x, y, b) end
      end
      if b.charge <= 0 and not b.held then row[x] = nil end
    end
  end
  if ghosts > 0 and tick % (ghosts * 8) == 0 then
    local b = bit(math.random(1, cols()), math.random(1, rows()))
    b.charge = math.random(4, 12)
  end
end

function grid_redraw()
  g:all(0)
  for y, row in pairs(bits) do
    for x, b in pairs(row) do g:led(x, y, b.held and math.max(b.charge, 3) or b.charge) end
  end
  g:refresh()
end

g.key = function(x, y, z)
  local b = bit(x, y)
  if z == 1 then
    -- a pulsing key goes quiet; a quiet one starts charging
    if b.charge > 0 and not b.held then
      b.charge = 0
    else
      b.held = true
    end
  else
    b.held = false
    b.t = 0
  end
  grid_redraw()
end

local function charge_three()
  for _ = 1, 3 do
    local b = bit(math.random(1, cols()), math.random(1, rows()))
    b.charge = math.random(6, 15)
  end
end

function init()
  scale = MusicUtil.generate_scale_of_length(root, 1, 64)
  engine.release(0.3)
  engine.cutoff(3000)
  charge_three()
  clock.run(function()
    while true do
      clock.sync(1 / 8)
      step()
      grid_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then ghosts = util.clamp(ghosts + d, 0, 32)
  elseif n == 3 then
    root = util.clamp(root + d, 30, 70)
    scale = MusicUtil.generate_scale_of_length(root, 1, 64)
  end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then bits = {} elseif n == 3 then charge_three() end
  grid_redraw()
end

function redraw()
  screen.clear()
  local w, h = math.floor(124 / cols()), math.floor(40 / rows())
  local live = 0
  for y, row in pairs(bits) do
    for x, b in pairs(row) do
      live = live + 1
      screen.level(math.max(b.charge, 1))
      screen.rect(2 + (x - 1) * w, 12 + (y - 1) * h, math.max(w - 1, 1), math.max(h - 1, 1))
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("charge")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(live .. " live")
  screen.move(0, 62)
  screen.text(ghosts == 0 and "no ghosts" or ("ghost every " .. ghosts))
  screen.update()
end
