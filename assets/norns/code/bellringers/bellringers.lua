-- bellringers
-- a Portamax norns script
--
-- six bells rung in changes:
-- each row is a new order of
-- the six, every bell moving
-- at most one place. the
-- treble's path weaves down
-- the page as it hunts.
--
-- E2 speed      E3 path bell
-- K2 call bob   K3 back to rounds
-- pads: ring a bell by hand
-- (params: method, key, bob chance)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 6
local METHODS = { "plain hunt", "plain bob", "bob ringing" }
local DEGREES = { 9, 7, 5, 4, 2, 0 } -- treble (1) high .. tenor (6) low

local row = {}
local history = {}
local change_no = 0
local lead_pos = 0
local bob_called = false
local bob_shown = 0
local striking = 0
local strike_flash = {}
local call_text = ""

local function rounds()
  row = {}
  for i = 1, N do row[i] = i end
  history = { { r = { table.unpack(row) }, mark = "rounds" } }
  change_no = 0
  lead_pos = 0
  bob_called = false
  call_text = "rounds"
end

-- apply place notation: "x" swaps every pair, otherwise the listed
-- places stay put and every other adjacent pair swaps
local function apply(pn)
  local keep = {}
  if pn ~= "x" then
    for c in string.gmatch(pn, "%d") do keep[tonumber(c)] = true end
  end
  local i = 1
  while i < N do
    if keep[i] then
      i = i + 1
    elseif keep[i + 1] then
      i = i + 1
    else
      row[i], row[i + 1] = row[i + 1], row[i]
      i = i + 2
    end
  end
end

local function next_change()
  local method = params:get("method")
  -- plain bob minor lead: x16x16x16x16x16x then 12 (plain) / 14 (bob)
  lead_pos = lead_pos % 12 + 1
  local pn
  if lead_pos % 2 == 1 then
    pn = "x"
  elseif lead_pos < 12 then
    pn = "16"
  else
    if method == 1 then pn = "16"
    elseif bob_called then pn = "14"
    else pn = "12" end
    if bob_called then bob_shown = 12 end
    bob_called = false
  end
  apply(pn)
  change_no = change_no + 1
  local mark = ""
  if lead_pos == 12 then mark = (pn == "14") and "bob" or "lead" end
  table.insert(history, { r = { table.unpack(row) }, mark = mark })
  while #history > 7 do table.remove(history, 1) end
  local is_rounds = true
  for i = 1, N do if row[i] ~= i then is_rounds = false end end
  if is_rounds then
    call_text = "that's all"
    lead_pos = 0
  elseif not bob_called then
    call_text = ""
  end
  -- the next lead might get a bob
  if lead_pos == 6 and method == 3 and math.random() < params:get("bob_chance") then
    bob_called = true
    call_text = "bob!"
  end
end

local function ring(bell, place)
  local note = params:get("key") + DEGREES[bell]
  engine.pan(util.linlin(1, N, -0.7, 0.7, place))
  engine.pw(0.5)
  engine.cutoff(2800)
  engine.release(1.8 + bell * 0.25)
  engine.amp(0.22)
  engine.hz(MusicUtil.note_num_to_freq(note))
  -- a quiet inharmonic partial (a minor third above the octave)
  engine.amp(0.06)
  engine.release(0.6)
  engine.hz(MusicUtil.note_num_to_freq(note + 15))
  strike_flash[bell] = 15
end

function init()
  params:add_separator("BELLRINGERS")
  params:add_option("method", "method", METHODS, 3)
  params:add_number("key", "tenor", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_number("speed", "strikes/min", 120, 600, 330)
  params:add_number("path", "path bell", 1, N, 2)
  params:add_control("bob_chance", "bob chance", controlspec.new(0, 1, 'lin', 0, 0.35, ''))
  params:default()
  engine.gain(1)
  math.randomseed(os.time())
  for i = 1, N do strike_flash[i] = 0 end
  rounds()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then ring((msg.note % N) + 1, 3.5) end
  end
  clock.run(function()
    local whole_pull = 0
    while true do
      -- strike the current row bell by bell
      for p = 1, N do
        striking = p
        ring(row[p], p)
        clock.sleep(60 / params:get("speed"))
      end
      striking = 0
      -- open handstroke lead: a one-beat gap after every backstroke
      whole_pull = whole_pull + 1
      if whole_pull % 2 == 0 then clock.sleep(60 / params:get("speed")) end
      next_change()
    end
  end)
  local frame = metro.init(function()
    for i = 1, N do strike_flash[i] = math.max(0, strike_flash[i] - 1) end
    bob_shown = math.max(0, bob_shown - 1)
    redraw()
  end, 1 / 20)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d * 5)
  elseif n == 3 then params:delta("path", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    bob_called = true
    call_text = "bob!"
  elseif n == 3 then
    rounds()
  end
  redraw()
end

local X0, DX, Y0, DY = 6, 11, 10, 8

function redraw()
  screen.clear()
  local pathbell = params:get("path")
  -- the treble's path, and the chosen bell's path
  for _, b in ipairs({ pathbell, 1 }) do
    screen.level(b == 1 and 10 or 3)
    screen.line_width(1)
    for r = 1, #history do
      local hr = history[r].r
      for p = 1, N do
        if hr[p] == b then
          local x, y = X0 + (p - 1) * DX + 2, Y0 + (r - 1) * DY - 3
          if r == 1 then screen.move(x, y) else screen.line(x, y) end
        end
      end
    end
    screen.stroke()
  end
  -- the rows as numbers; the newest at the bottom
  for r = 1, #history do
    local hr = history[r].r
    local newest = (r == #history)
    for p = 1, N do
      local b = hr[p]
      local lvl = newest and 6 or 3
      if b == 1 then lvl = 15 elseif b == pathbell then lvl = 9 end
      if newest and p == striking then lvl = 15 end
      screen.level(lvl)
      screen.move(X0 + (p - 1) * DX, Y0 + (r - 1) * DY)
      screen.text(tostring(b))
    end
    if history[r].mark ~= "" then
      screen.level(2)
      screen.move(X0 + N * DX, Y0 + (r - 1) * DY)
      screen.text(history[r].mark == "bob" and "-" or (history[r].mark == "rounds" and "" or "."))
    end
  end
  -- bells, swinging with each strike
  for b = 1, N do
    local x = 84 + ((b - 1) % 3) * 15
    local y = 26 + math.floor((b - 1) / 3) * 17
    local s = strike_flash[b] / 15
    screen.level(math.max(3, strike_flash[b]))
    screen.move(x - 4 + s * 2, y + 4)
    screen.line(x - 2 + s * 2, y - 4)
    screen.line(x + 2 + s * 2, y - 4)
    screen.line(x + 4 + s * 2, y + 4)
    screen.close()
    if strike_flash[b] > 8 then screen.fill() else screen.stroke() end
    screen.level(4)
    screen.move(x, y + 11)
    screen.text_center(tostring(b))
  end
  screen.level(15)
  screen.move(127, 8)
  screen.text_right("bellringers")
  screen.level(5)
  screen.move(127, 17)
  screen.text_right(METHODS[params:get("method")])
  screen.level(bob_shown > 0 and 15 or 6)
  screen.move(84, 63)
  screen.text(bob_shown > 0 and "bob!" or call_text)
  screen.level(3)
  screen.move(127, 63)
  screen.text_right(change_no)
  screen.update()
end
