-- stacks
-- a Portamax norns script
--
-- blocks fall into eight tuned
-- columns. each one rings its
-- column's note as it lands.
-- fill a whole row and it clears
-- with a chord.
--
-- E2 fall speed   E3 aim
-- K2 drop now   K3 clear all
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local COLS, ROWS = 8, 10
local heap = {}
local falling = nil
local aim = 4
local scale = {}
local cleared = 0
local flash = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), COLS)
end

local function height(c)
  return #heap[c]
end

local function spawn()
  local c = math.random() < 0.5 and aim or math.random(1, COLS)
  falling = { c = c, y = 0 }
end

local function land(c)
  table.insert(heap[c], true)
  engine.pan((c - 4.5) / 4)
  engine.hz(MusicUtil.note_num_to_freq(scale[c] + height(c) % 3 * 12))
  -- a full bottom row clears with a chord of every column
  local full = true
  for i = 1, COLS do if height(i) == 0 then full = false end end
  if full then
    for i = 1, COLS do table.remove(heap[i], 1) end
    for i = 1, COLS, 2 do engine.hz(MusicUtil.note_num_to_freq(scale[i])) end
    cleared = cleared + 1
    flash = 10
  end
  for i = 1, COLS do if height(i) >= ROWS then heap[i] = {} end end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("STACKS")
  params:add_number("speed", "fall speed", 1, 8, 3)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.7)
  engine.amp(0.25)
  engine.cutoff(2800)
  for i = 1, COLS do heap[i] = {} end
  math.randomseed(os.time())
  build_scale()
  spawn()
  local m = metro.init(function()
    falling.y = falling.y + params:get("speed") * 0.05
    if falling.y >= ROWS - height(falling.c) then
      land(falling.c)
      spawn()
    end
    flash = math.max(0, flash - 1)
    redraw()
  end, 1 / 30)
  m:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then aim = util.clamp(aim + d, 1, COLS) falling.c = aim end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then falling.y = ROWS
  elseif n == 3 then for i = 1, COLS do heap[i] = {} end end
end

function redraw()
  screen.clear()
  local cw, rh = 10, 5
  local x0 = 24
  if flash > 0 then
    screen.level(flash)
    screen.rect(x0, 60, COLS * cw, 2)
    screen.fill()
  end
  for c = 1, COLS do
    screen.level(c == aim and 4 or 1)
    screen.rect(x0 + (c - 1) * cw, 10, cw - 1, ROWS * rh)
    screen.stroke()
    for r = 1, height(c) do
      screen.level(6 + r % 3 * 3)
      screen.rect(x0 + (c - 1) * cw + 1, 10 + (ROWS - r) * rh + 1, cw - 3, rh - 2)
      screen.fill()
    end
  end
  screen.level(15)
  screen.rect(x0 + (falling.c - 1) * cw + 1, 10 + math.floor(falling.y) * rh + 1, cw - 3, rh - 2)
  screen.fill()
  screen.move(0, 7)
  screen.text("stacks")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right("rows " .. cleared)
  screen.update()
end
