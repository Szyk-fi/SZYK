-- stutter
-- a Portamax norns script
--
-- a busy little groove plays while
-- softcut keeps the most recent
-- beat on tape. press K3 and, on
-- the next beat, the live part
-- drops out and that beat repeats
-- in ever-smaller slices.
--
-- E2 slice size   E3 stutter pitch
-- K2 new groove   K3 stutter on/off
-- (params: tone, shrink)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local DIVS = { 1, 2, 3, 4, 6, 8 }
local groove = {}
local pos = 0
local want = false
local stuttering = false
local slice_n = 0
local beat_len = 0.5

local function new_groove()
  local s = MusicUtil.generate_scale_of_length(45, "Dorian", 12)
  groove = {}
  for i = 1, 8 do
    groove[i] = (i % 2 == 1 or math.random() < 0.6) and s[math.random(#s)] or 0
  end
  groove[1] = s[1]
end

local function setup()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  -- voice 1 writes the current beat, voice 2 replays a slice of it
  for v = 1, 2 do
    softcut.enable(v, 1)
    softcut.buffer(v, 1)
    softcut.loop(v, 1)
    softcut.loop_start(v, 1)
    softcut.loop_end(v, 1 + beat_len)
    softcut.position(v, 1)
    softcut.fade_time(v, 0.005)
    softcut.play(v, 1)
  end
  softcut.level(1, 0)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0)
  softcut.rec(1, 1)
  softcut.level(2, 0)
  softcut.rec(2, 0)
end

local function apply_stutter()
  local div = DIVS[params:get("div")]
  softcut.rec(1, 0)
  softcut.loop_start(2, 1)
  softcut.loop_end(2, 1 + beat_len / div)
  softcut.position(2, 1)
  softcut.rate(2, 2 ^ (params:get("pitch") / 12))
  softcut.level(2, 0.9)
end

local function release_stutter()
  softcut.level(2, 0)
  softcut.rate(2, 1)
  softcut.rec(1, 1)
end

function init()
  params:add_separator("STUTTER")
  params:add_option("div", "slice", { "1", "1/2", "1/3", "1/4", "1/6", "1/8" }, 3)
  params:set_action("div", function() if stuttering then apply_stutter() end end)
  params:add_number("pitch", "stutter pitch", -12, 12, 0)
  params:set_action("pitch", function(x) softcut.rate(2, 2 ^ (x / 12)) end)
  params:add_binary("shrink", "shrink slices", "toggle", 1)
  params:add_control("tone", "tone", controlspec.new(300, 5000, 'exp', 0, 1600, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(0.25)
  engine.amp(0.28)
  math.randomseed(os.time())
  new_groove()
  beat_len = 60 / clock.get_tempo()
  setup()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      pos = pos % 8 + 1
      if pos % 2 == 1 then
        -- on the beat: decide, and line the record head up with the beat
        if want and not stuttering then stuttering = true slice_n = 0 apply_stutter()
        elseif not want and stuttering then stuttering = false release_stutter() end
        if stuttering then
          slice_n = slice_n + 1
          if params:get("shrink") == 1 and slice_n % 2 == 0 and params:get("div") < #DIVS then
            softcut.loop_end(2, 1 + beat_len / DIVS[params:get("div")] / (1 + slice_n // 2))
          end
        else
          softcut.position(1, 1)
        end
      end
      local n = groove[pos]
      if n > 0 and not stuttering then
        engine.pw(pos % 2 == 1 and 0.5 or 0.25)
        engine.hz(MusicUtil.note_num_to_freq(n + (pos == 5 and 12 or 0)))
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("div", d)
  elseif n == 3 then params:delta("pitch", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_groove()
  elseif n == 3 then want = not want end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, 8 do
    local n = groove[i]
    screen.level(i == pos and 15 or 4)
    if n > 0 then
      screen.rect(4 + (i - 1) * 15, 40 - (n - 45), 12, 3)
      screen.fill()
    end
  end
  if stuttering then
    local div = DIVS[params:get("div")]
    for k = 1, math.min(div * (1 + slice_n // 2), 32) do
      screen.level(k % 2 == 0 and 15 or 8)
      screen.rect(4 + (k - 1) * 120 / math.min(div * (1 + slice_n // 2), 32), 48, 2, 6)
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(stuttering and "stutter !!" or (want and "stutter (armed)" or "stutter"))
  screen.level(4)
  screen.move(0, 62)
  screen.text("slice " .. params:string("div"))
  screen.move(127, 62)
  screen.text_right("pitch " .. params:get("pitch"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
