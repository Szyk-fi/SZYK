-- grains
-- a Portamax norns script
--
-- a short phrase is written into a
-- three-second stretch of tape. two
-- grain heads leap around inside it,
-- each looping a tiny window at its
-- own speed and pan, smearing the
-- phrase into a cloud.
--
-- E2 grain size   E3 spray
-- K2 new phrase   K3 freeze tape
-- (params: phrase level, pitch mix)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local REG_S, REG_E = 1, 4
local phrase = {}
local pstep = 0
local frozen = false
local heads = { { p = 2, r = 1 }, { p = 3, r = 1 } }
local recpos = 0
local RATES = { 1, 1, 0.5, 2, -1 }

local function new_phrase()
  local s = MusicUtil.generate_scale_of_length(62, "Lydian", 12)
  phrase = {}
  for i = 1, 8 do phrase[i] = math.random() < 0.2 and 0 or s[math.random(#s)] end
  phrase[1] = s[1]
end

local function setup()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  -- voice 1: the recorder
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0)
  softcut.loop(1, 1)
  softcut.loop_start(1, REG_S)
  softcut.loop_end(1, REG_E)
  softcut.position(1, REG_S)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 0.9)
  softcut.pre_level(1, 0.4)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  -- voices 2, 3: grain heads
  for v = 2, 3 do
    softcut.enable(v, 1)
    softcut.buffer(v, 1)
    softcut.level(v, 0.9)
    softcut.loop(v, 1)
    softcut.fade_time(v, 0.02)
    softcut.play(v, 1)
    softcut.rec(v, 0)
    softcut.loop_start(v, 2)
    softcut.loop_end(v, 2.1)
    softcut.position(v, 2)
  end
end

local function grain(h)
  local v = h + 1
  local size = params:get("size")
  local spread = params:get("spray") * (REG_E - REG_S - size)
  local centre = (REG_S + REG_E - size) / 2
  local p = util.clamp(centre + (math.random() * 2 - 1) * spread / 2, REG_S, REG_E - size)
  local r = math.random() < params:get("pitchmix") and RATES[math.random(#RATES)] or 1
  heads[h] = { p = p, r = r }
  softcut.loop_start(v, p)
  softcut.loop_end(v, p + size)
  softcut.position(v, r > 0 and p or p + size)
  softcut.rate(v, r)
  softcut.pan(v, (math.random() - 0.5) * 1.6)
end

function init()
  params:add_separator("GRAINS")
  params:add_control("size", "grain size", controlspec.new(0.03, 0.5, 'exp', 0, 0.12, 's'))
  params:add_control("spray", "spray", controlspec.new(0, 1, 'lin', 0, 0.6, ''))
  params:add_control("pitchmix", "pitch mix", controlspec.new(0, 1, 'lin', 0, 0.3, ''))
  params:add_control("dry", "phrase level", controlspec.new(0.05, 0.4, 'lin', 0, 0.18, ''))
  params:set_action("dry", function(x) engine.amp(x) end)
  params:default()
  engine.release(0.9)
  engine.cutoff(2200)
  math.randomseed(os.time())
  new_phrase()
  setup()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      pstep = pstep % #phrase + 1
      local n = phrase[pstep]
      if n > 0 then engine.hz(MusicUtil.note_num_to_freq(n)) end
    end
  end)
  for h = 1, 2 do
    clock.run(function()
      clock.sleep(h * 0.13)
      while true do
        grain(h)
        clock.sleep(params:get("size") * (1.5 + math.random()))
      end
    end)
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      if not frozen then recpos = (recpos + 1 / 15) % (REG_E - REG_S) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("size", d)
  elseif n == 3 then params:delta("spray", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_phrase()
  elseif n == 3 then
    frozen = not frozen
    softcut.rec(1, frozen and 0 or 1)
  end
end

function redraw()
  screen.clear()
  local function x_of(t) return 4 + (t - REG_S) / (REG_E - REG_S) * 120 end
  screen.level(2)
  screen.rect(4, 30, 120, 8)
  screen.stroke()
  for h = 1, 2 do
    local g = heads[h]
    screen.level(h == 1 and 15 or 9)
    screen.rect(x_of(g.p), 22 + h * 4, math.max(1, params:get("size") / 3 * 120), 3)
    screen.fill()
    screen.move(x_of(g.p), 20 + h * 14)
    screen.text(g.r == 1 and "" or (g.r < 0 and "<" or (g.r > 1 and "^" or "v")))
  end
  if not frozen then
    screen.level(6)
    screen.move(4 + recpos / (REG_E - REG_S) * 120, 40)
    screen.line_rel(0, 6)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(frozen and "grains (frozen)" or "grains")
  screen.level(4)
  screen.move(0, 62)
  screen.text("size " .. params:string("size"))
  screen.move(127, 62)
  screen.text_right("spray " .. string.format("%.2f", params:get("spray")))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
