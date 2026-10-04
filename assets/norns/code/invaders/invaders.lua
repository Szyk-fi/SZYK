-- invaders
-- a Portamax norns script
--
-- rows of invaders march and
-- drop; every step they take is
-- a note of a falling four-note
-- bass line, quicker as their
-- numbers thin. the cannon
-- plays itself.
--
-- E2 march speed  E3 bass key
-- K2 new wave     K3 fire
-- pads: fire, pitched
-- (params: motif, ping level)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local COLS, ROWS = 8, 4
local MOTIFS = {
  { name = "minor", iv = { 0, -2, -3, -5 } },
  { name = "chromatic", iv = { 0, -1, -2, -3 } },
  { name = "fourths", iv = { 0, -5, -7, -12 } },
  { name = "whole", iv = { 0, -2, -4, -6 } },
}
-- two frames of a small 7x5 sprite, row by row
local SPRITES = {
  { { "..x.x..", ".xxxxx.", "xx.x.xx", "xxxxxxx", "x.x.x.x" }, { "..x.x..", ".xxxxx.", "xx.x.xx", "xxxxxxx", ".x...x." } },
  { { ".x...x.", "..xxx..", ".xxxxx.", "x.xxx.x", "..x.x.." }, { ".x...x.", "x.xxx.x", "xxxxxxx", "..xxx..", ".x...x." } },
}

local alive = {}
local ox, oy, dir = 8, 10, 1
local march_step = 0
local frame_t = 0
local since_march = 0
local cannon = 64
local target_x = 64
local target_col = nil
local aim_err = 0
local shots = {}
local bombs = {}
local booms = {}
local wave = 1
local score = 0
local cooldown = 0

local function count()
  local n = 0
  for r = 1, ROWS do for c = 1, COLS do if alive[r][c] then n = n + 1 end end end
  return n
end

local function new_wave()
  for r = 1, ROWS do
    alive[r] = {}
    for c = 1, COLS do alive[r][c] = true end
  end
  ox, oy, dir = 8, 10, 1
  shots, bombs = {}, {}
end

local function tone(hz, amp, rel, cut, pw, pan)
  engine.pan(pan or 0)
  engine.pw(pw or 0.5)
  engine.cutoff(cut or 2000)
  engine.release(rel or 0.2)
  engine.amp(amp)
  engine.hz(hz)
end

local function march_note()
  local m = MOTIFS[params:get("motif")].iv
  local n = params:get("bass") + m[march_step % 4 + 1]
  tone(MusicUtil.note_num_to_freq(n), 0.5, 0.22, 420, 0.5, 0)
end

local function fire(pitch)
  if #shots >= 2 then return end
  table.insert(shots, { x = cannon, y = 54 })
  local n = pitch or (84 + math.random(0, 4))
  tone(MusicUtil.note_num_to_freq(n), 0.18 * params:get("ping"), 0.12, 6000, 0.2, util.linlin(0, 127, -0.6, 0.6, cannon))
end

local function inv_pos(r, c)
  return ox + (c - 1) * 11, oy + (r - 1) * 8
end

local function march()
  -- edge check over the live invaders only
  local minx, maxx, maxy = 999, -1, 0
  for r = 1, ROWS do
    for c = 1, COLS do
      if alive[r][c] then
        local x, y = inv_pos(r, c)
        minx, maxx, maxy = math.min(minx, x), math.max(maxx, x + 7), math.max(maxy, y + 5)
      end
    end
  end
  if (dir > 0 and maxx + 2 > 127) or (dir < 0 and minx - 2 < 0) then
    dir = -dir
    oy = oy + 2
  else
    ox = ox + dir * 2
  end
  march_note()
  march_step = march_step + 1
  frame_t = 1 - frame_t
  -- a random live invader in the bottom of its column drops a bomb
  if math.random() < 0.25 then
    local c = math.random(COLS)
    for r = ROWS, 1, -1 do
      if alive[r][c] then
        local x, y = inv_pos(r, c)
        table.insert(bombs, { x = x + 3, y = y + 6 })
        break
      end
    end
  end
  if maxy >= 54 then
    -- they landed: a low growl and a fresh wave
    for i = 0, 2 do tone(MusicUtil.note_num_to_freq(params:get("bass") - 12 - i), 0.4, 1.2, 300, 0.3, 0) end
    wave = 1
    score = 0
    new_wave()
  end
end

local function step(dt)
  local n = count()
  if n == 0 then
    tone(MusicUtil.note_num_to_freq(params:get("bass") + 24), 0.2, 1, 3000, 0.5, -0.3)
    tone(MusicUtil.note_num_to_freq(params:get("bass") + 28), 0.2, 1, 3000, 0.5, 0.3)
    tone(MusicUtil.note_num_to_freq(params:get("bass") + 31), 0.2, 1, 3000, 0.5, 0)
    wave = wave + 1
    new_wave()
    n = count()
  end
  -- the fewer remain, the faster they march
  local interval = params:get("speed") * (0.25 + 0.75 * n / (ROWS * COLS))
  since_march = since_march + dt
  if since_march >= interval then
    since_march = 0
    march()
  end
  -- autopilot: pick a column, stay under it as the rank marches
  local col_alive = false
  if target_col then
    for r = 1, ROWS do if alive[r][target_col] then col_alive = true end end
  end
  if not col_alive or math.random() < 0.005 then
    local cs = {}
    for c = 1, COLS do
      for r = 1, ROWS do if alive[r][c] then cs[#cs + 1] = c break end end
    end
    target_col = cs[math.random(#cs)]
    -- the autopilot is a fair shot, not a perfect one
    aim_err = math.random(-6, 6)
  end
  target_x = ox + (target_col - 1) * 11 + 3 + dir * 2 + aim_err
  for _, b in ipairs(bombs) do
    if b.y > 38 and math.abs(b.x - cannon) < 5 then target_x = cannon + (b.x < cannon and 12 or -12) end
  end
  target_x = util.clamp(target_x, 4, 123)
  cannon = cannon + util.clamp(target_x - cannon, -60 * dt, 60 * dt)
  cooldown = cooldown - dt
  if cooldown <= 0 and math.abs(target_x - cannon) < 3 then
    fire()
    if math.random() < 0.5 then aim_err = math.random(-4, 4) end
    cooldown = 0.35 + math.random() * 0.3
  end
  for i = #shots, 1, -1 do
    local s = shots[i]
    s.y = s.y - 90 * dt
    local hit = false
    for r = ROWS, 1, -1 do
      for c = 1, COLS do
        if not hit and alive[r][c] then
          local x, y = inv_pos(r, c)
          if s.x >= x and s.x <= x + 7 and s.y >= y and s.y <= y + 5 then
            alive[r][c] = false
            hit = true
            score = score + (ROWS - r + 1) * 10
            table.insert(booms, { x = x + 3, y = y + 2, life = 8 })
            -- the hit: a short bright chirp a step above the shot
            tone(MusicUtil.note_num_to_freq(91 + r), 0.14 * params:get("ping"), 0.07, 9000, 0.1, 0)
          end
        end
      end
    end
    if hit or s.y < 0 then table.remove(shots, i) end
  end
  for i = #bombs, 1, -1 do
    local b = bombs[i]
    b.y = b.y + 30 * dt
    if b.y > 56 and math.abs(b.x - cannon) < 4 then
      table.insert(booms, { x = cannon, y = 57, life = 12 })
      tone(MusicUtil.note_num_to_freq(params:get("bass") - 5), 0.4, 0.5, 600, 0.2, 0)
      table.remove(bombs, i)
    elseif b.y > 64 then
      table.remove(bombs, i)
    end
  end
  for i = #booms, 1, -1 do
    booms[i].life = booms[i].life - 1
    if booms[i].life <= 0 then table.remove(booms, i) end
  end
end

function init()
  local names = {}
  for i, m in ipairs(MOTIFS) do names[i] = m.name end
  params:add_separator("INVADERS")
  params:add_control("speed", "march step", controlspec.new(0.12, 1.2, 'exp', 0, 0.42, 's'))
  params:add_number("bass", "bass key", 28, 52, 40, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_option("motif", "motif", names, 1)
  params:add_control("ping", "ping level", controlspec.new(0, 2, 'lin', 0, 1, ''))
  params:default()
  engine.gain(1.5)
  math.randomseed(os.time())
  new_wave()
  march()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      cooldown = 0.3
      fire(msg.note + 24)
    end
  end
  local last = util.time()
  local frame = metro.init(function()
    local now = util.time()
    step(math.min(0.1, now - last))
    last = now
    redraw()
  end, 1 / 40)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", -d)
  elseif n == 3 then params:delta("bass", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    wave = 1
    score = 0
    new_wave()
  elseif n == 3 then
    fire()
  end
  redraw()
end

local function sprite(rows, x, y)
  for j, line in ipairs(rows) do
    for i = 1, #line do
      if string.sub(line, i, i) == "x" then screen.rect(x + i - 1, y + j - 1, 1, 1) end
    end
  end
  screen.fill()
end

function redraw()
  screen.clear()
  for r = 1, ROWS do
    screen.level(({ 15, 11, 8, 6 })[r])
    local kind = (r <= 2) and 2 or 1
    for c = 1, COLS do
      if alive[r][c] then
        local x, y = inv_pos(r, c)
        sprite(SPRITES[kind][frame_t + 1], math.floor(x), math.floor(y))
      end
    end
  end
  screen.level(15)
  for _, s in ipairs(shots) do
    screen.move(s.x, s.y)
    screen.line(s.x, s.y + 3)
    screen.stroke()
  end
  screen.level(7)
  for _, b in ipairs(bombs) do
    screen.rect(b.x, b.y, 1, 3)
    screen.fill()
  end
  for _, b in ipairs(booms) do
    screen.level(math.min(15, b.life * 2))
    screen.circle(b.x, b.y, 9 - b.life / 2)
    screen.stroke()
  end
  -- the cannon
  screen.level(12)
  local cx = math.floor(cannon)
  screen.rect(cx - 4, 58, 9, 3)
  screen.fill()
  screen.rect(cx, 56, 1, 2)
  screen.fill()
  screen.level(3)
  screen.move(0, 63)
  screen.line(127, 63)
  screen.stroke()
  screen.level(5)
  screen.move(0, 6)
  screen.text(score)
  screen.move(127, 6)
  screen.text_right("wave " .. wave)
  screen.update()
end
