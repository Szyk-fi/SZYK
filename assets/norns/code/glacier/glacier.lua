-- glacier
-- a Portamax norns script
--
-- a river of ice creeping down a
-- valley. a slow drone shifts one
-- chord tone at a time, so the
-- harmony creeps too. now and then
-- the ice cracks: a sharp, bright
-- splinter of notes that rings out
-- into the cold echo.
--
-- E2 creep   E3 crack rate
-- K2 crack now   K3 freeze
-- pads: drop a stone in a crevasse
-- (params: scale, root, echo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local voices = { 1, 3, 5, 8 } -- scale degrees of the drone chord
local frozen = false
local flow = 0
local cracks = {}
local voice_i = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function drone()
  voice_i = voice_i % #voices + 1
  engine.pan(({ -0.5, 0.3, -0.2, 0.5 })[voice_i])
  engine.pw(0.5)
  engine.cutoff(params:get("tone"))
  engine.release(6)
  engine.amp(0.2)
  engine.hz(MusicUtil.note_num_to_freq(scale[voices[voice_i]]))
end

-- one voice of the chord moves a step: the harmony creeps
local function creep()
  local i = math.random(1, #voices)
  local nv = util.clamp(voices[i] + (math.random() < 0.5 and -1 or 1), 1, 14)
  if not tab.contains(voices, nv) then voices[i] = nv end
end

local function crack()
  table.insert(cracks, { x = 20 + math.random() * 90, len = 4 + math.random() * 10, life = 30 })
  clock.run(function()
    for i = 1, math.random(2, 4) do
      engine.pan(math.random() - 0.5)
      engine.pw(0.08)
      engine.cutoff(7000)
      engine.release(0.15 + math.random() * 0.4)
      engine.amp(0.15)
      engine.hz(MusicUtil.note_num_to_freq(scale[math.random(14, #scale)] + 12))
      clock.sleep(0.02 + math.random() * 0.06)
    end
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("GLACIER")
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 33, 57, 43, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("creep", "creep", controlspec.new(0.1, 4, 'exp', 0, 1, 'x'))
  params:add_control("cracks", "crack rate", controlspec.new(0, 1, 'lin', 0.01, 0.25, ''))
  params:add_control("tone", "drone tone", controlspec.new(200, 3000, 'exp', 0, 700, 'hz'))
  params:add_control("echo", "echo", controlspec.new(0, 0.9, 'lin', 0, 0.7, ''))
  params:set_action("echo", function(x) softcut.pre_level(1, x) end)
  audio.level_eng_cut(0.5)
  softcut.buffer_clear()
  softcut.enable(1, 1) softcut.buffer(1, 1) softcut.level(1, 0.5) softcut.rate(1, 1)
  softcut.loop(1, 1) softcut.loop_start(1, 1) softcut.loop_end(1, 2.3) softcut.position(1, 1)
  softcut.level_input_cut(1, 1, 1) softcut.level_input_cut(2, 1, 1)
  softcut.rec_level(1, 1) softcut.play(1, 1) softcut.rec(1, 1)
  softcut.filter_dry(1, 0) softcut.filter_lp(1, 1) softcut.filter_fc(1, 3000)
  params:default()
  math.randomseed(os.time())
  build_scale()
  drone()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then crack() end
  end
  clock.run(function()
    local beats = 0
    while true do
      clock.sleep(1.6 / params:get("creep") ^ 0.5)
      if not frozen then
        beats = beats + 1
        drone()
        if beats % 6 == 0 then creep() end
        if math.random() < params:get("cracks") * 0.3 then crack() end
      end
    end
  end)
  metro.init(function()
    if not frozen then flow = flow + params:get("creep") * 0.05 end
    for i = #cracks, 1, -1 do
      cracks[i].life = cracks[i].life - 1
      if cracks[i].life <= 0 then table.remove(cracks, i) end
    end
    redraw()
  end, 1 / 20):start()
end

function enc(n, d)
  if n == 2 then params:delta("creep", d)
  elseif n == 3 then params:delta("cracks", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then crack()
  elseif n == 3 then frozen = not frozen end
end

function redraw()
  screen.clear()
  -- valley walls
  screen.level(3)
  screen.move(0, 14) screen.line(127, 20) screen.stroke()
  screen.move(0, 54) screen.line(127, 48) screen.stroke()
  -- flow bands creeping down-valley
  for i = 0, 9 do
    local x = (i * 14 + flow) % 140 - 6
    screen.level(i % 3 == 0 and 5 or 2)
    screen.move(x, 16 + x / 21)
    screen.curve(x + 6, 26, x + 6, 42, x, 52 - x / 21)
    screen.stroke()
  end
  for _, c in ipairs(cracks) do
    screen.level(math.floor(c.life / 2))
    screen.move(c.x, 30)
    screen.line(c.x + 2, 30 + c.len * 0.5)
    screen.line(c.x - 1, 30 + c.len)
    screen.stroke()
  end
  -- the drone chord as four bars
  for i, v in ipairs(voices) do
    screen.level(i == voice_i and 12 or 4)
    screen.rect(96 + i * 6, 40 - v, 4, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(frozen and "glacier (frozen)" or "glacier")
  screen.level(4)
  screen.move(0, 62)
  screen.text("creep " .. params:string("creep"))
  screen.move(127, 62)
  screen.text_right("cracks " .. params:string("cracks"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
