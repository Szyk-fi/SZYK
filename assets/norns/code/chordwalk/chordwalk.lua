-- chordwalk
-- a Portamax norns script
--
-- an arpeggiator that walks a
-- chord progression. the pads
-- pick the key; the progression
-- and the arp shape are yours.
--
-- E2 progression   E3 shape
-- K2 next chord   K3 run / stop
-- (params: tempo, spread)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PROGS = {
  { name = "I V vi IV", steps = { { 0, "major" }, { 7, "major" }, { 9, "minor" }, { 5, "major" } } },
  { name = "i VI III VII", steps = { { 0, "minor" }, { 8, "major" }, { 3, "major" }, { 10, "major" } } },
  { name = "ii V I", steps = { { 2, "minor 7" }, { 7, "dominant 7" }, { 0, "major 7" }, { 0, "major 7" } } },
  { name = "i iv v i", steps = { { 0, "minor" }, { 5, "minor" }, { 7, "minor" }, { 0, "minor" } } },
}
local SHAPES = { "up", "down", "updown", "random", "outside" }
local key_root = 48
local chord_i = 1
local step = 0
local running = true
local notes = {}
local lit = 0

local function build()
  local s = PROGS[params:get("prog")].steps[chord_i]
  local c = MusicUtil.generate_chord(key_root + s[1], s[2], 0)
  notes = {}
  for o = 0, params:get("spread") - 1 do
    for _, n in ipairs(c) do notes[#notes + 1] = n + 12 * o end
  end
end

local function index(k, n)
  local shape = SHAPES[params:get("shape")]
  if shape == "up" then return (k - 1) % n + 1
  elseif shape == "down" then return n - (k - 1) % n
  elseif shape == "updown" then
    local p = (k - 1) % math.max(1, 2 * n - 2)
    return p < n and p + 1 or 2 * n - 1 - p
  elseif shape == "outside" then
    local p = (k - 1) % n
    return p % 2 == 0 and p // 2 + 1 or n - p // 2
  end
  return math.random(n)
end

function init()
  local prog_names = {}
  for i, p in ipairs(PROGS) do prog_names[i] = p.name end
  params:add_separator("CHORDWALK")
  params:add_option("prog", "progression", prog_names, 1)
  params:set_action("prog", build)
  params:add_option("shape", "shape", SHAPES, 3)
  params:add_number("spread", "octaves", 1, 3, 2)
  params:set_action("spread", build)
  params:default()
  engine.release(0.5)
  engine.cutoff(2400)
  engine.amp(0.25)
  build()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then key_root = msg.note - 12 build() end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if running then
        step = step + 1
        if step > 16 then
          step = 1
          chord_i = chord_i % 4 + 1
          build()
        end
        lit = index(step, #notes)
        engine.pan(lit / #notes - 0.5)
        engine.hz(MusicUtil.note_num_to_freq(notes[lit]))
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("prog", d)
  elseif n == 3 then params:delta("shape", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then chord_i = chord_i % 4 + 1 step = 0 build()
  elseif n == 3 then running = not running end
  redraw()
end

function redraw()
  screen.clear()
  for i, n in ipairs(notes) do
    screen.level(i == lit and 15 or 3)
    screen.rect(4 + (i - 1) * (120 / #notes), 52 - (n - key_root) * 1.2, 120 / #notes - 2, 3)
    screen.fill()
  end
  for c = 1, 4 do
    screen.level(c == chord_i and 15 or 3)
    screen.rect(84 + c * 9, 2, 7, 5)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text(PROGS[params:get("prog")].name)
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(key_root + 12, false) .. " " .. SHAPES[params:get("shape")])
  screen.move(127, 62)
  screen.text_right(running and "" or "stopped")
  screen.update()
end
