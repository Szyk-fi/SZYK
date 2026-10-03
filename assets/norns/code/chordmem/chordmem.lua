-- chordmem
-- a Portamax norns script
--
-- hold a few pads together and the
-- shape is memorised. the shape is
-- then carried through a chord
-- progression, transposed onto each
-- root and played broken or block.
-- it starts with a seventh chord.
--
-- E2 progression   E3 octave
-- K2 back to default   K3 broken / block
-- (params: tone, ring)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PROGS = {
  { name = "0 5 -3 7", r = { 0, 5, -3, 7 } },
  { name = "0 -2 -5 -7", r = { 0, -2, -5, -7 } },
  { name = "0 3 8 10", r = { 0, 3, 8, 10 } },
  { name = "0 4 -1 2", r = { 0, 4, -1, 2 } },
}
local DEFAULT = { 0, 4, 7, 11 }
local shape = {}
local held = {}
local chord_i = 1
local step = 0
local broken = true
local lit = 0

local function capture()
  local notes = {}
  for n in pairs(held) do notes[#notes + 1] = n end
  -- a single pad is just a note; a shape needs two or more held
  if #notes < 2 then return end
  table.sort(notes)
  shape = {}
  for i, n in ipairs(notes) do shape[i] = n - notes[1] end
end

local function voicing()
  local root = 48 + 12 * params:get("oct") + PROGS[params:get("prog")].r[chord_i]
  local out = {}
  for i, iv in ipairs(shape) do out[i] = root + iv end
  return out
end

local function play(n, amp)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

function init()
  local names = {}
  for i, p in ipairs(PROGS) do names[i] = p.name end
  params:add_separator("CHORDMEM")
  params:add_option("prog", "progression", names, 1)
  params:add_number("oct", "octave", -1, 1, 0)
  params:add_control("tone", "tone", controlspec.new(400, 5000, 'exp', 0, 1600, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("ring", "ring", controlspec.new(0.3, 3, 'lin', 0, 1.3, 's'))
  params:set_action("ring", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.4)
  for i, v in ipairs(DEFAULT) do shape[i] = v end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      held[msg.note] = true
      capture()
      play(msg.note, 0.2)
    elseif msg.type == "note_off" then
      held[msg.note] = nil
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      step = step % 8 + 1
      if step == 1 then chord_i = chord_i % 4 + 1 end
      local v = voicing()
      if broken then
        lit = (step - 1) % #v + 1
        play(v[lit], step == 1 and 0.28 or 0.2)
      elseif step == 1 or step == 4 or step == 7 then
        lit = 0
        for _, n in ipairs(v) do play(n, 0.12) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("prog", d)
  elseif n == 3 then params:delta("oct", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    shape = {}
    for i, v in ipairs(DEFAULT) do shape[i] = v end
  elseif n == 3 then broken = not broken end
  redraw()
end

function redraw()
  screen.clear()
  -- the memorised shape, one dot per semitone above its root
  for i, iv in ipairs(shape) do
    screen.level(i == lit and 15 or 6)
    screen.rect(6 + iv * 3, 46 - i * 6, 4, 4)
    screen.fill()
  end
  screen.level(2)
  screen.move(6, 50)
  screen.line(78, 50)
  screen.stroke()
  local r = PROGS[params:get("prog")].r
  for c = 1, 4 do
    local x = 88 + (c - 1) * 10
    screen.level(c == chord_i and 15 or 3)
    screen.rect(x, 40 - r[c], 7, 3)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("chordmem")
  screen.move(127, 8)
  screen.text_right(MusicUtil.note_num_to_name(voicing()[1]) .. " +" .. (#shape - 1))
  screen.level(4)
  screen.move(0, 62)
  screen.text(PROGS[params:get("prog")].name)
  screen.move(127, 62)
  screen.text_right(broken and "broken" or "block")
  screen.update()
end
