# Portamax neural models

The five AI apps each run a small network made for the STM32N6's
Neural-ART NPU. This folder trains them, quantises them to int8 and
exports them. Everything is trained on data synthesized by these scripts,
so nothing here comes from a dataset.

| Model | App | Input | Output | MACs | Script |
|---|---|---|---|---|---|
| `hum` | Hum | 64 ms log-frequency spectrum (216 bins) | 145 pitches (1/3 semitone, C2–C6) + no pitch | 817k, 100×/s | `train_hum.py` |
| `mouth` | Mouth Drums | 80 ms mel spectrogram (32×12) | 64-number sound embedding | 160k per sound | `train_mouth.py` |
| `gesture` | Conductor | 0.8 s of both depth sensors (2×48) | 8 gestures | 119k, 20×/s | `train_gesture.py` |
| `timbre` | Timbre Map | map point, note, velocity, time | 32 harmonics, 4 noise bands, loudness | ~30k per voice, 250×/s | `train_timbre.py` |
| `chords` | Band Mate | 256 ms energy per semitone (72 bins) | 24 chords + no chord | ~190k, 10×/s | `train_chords.py` |

## Rebuilding

```sh
pip install torch onnx numpy
cd tools/npu
python3 train_hum.py      # writes ../../assets/npu/hum.{pmxn,onnx,test.json,features.json}
python3 train_mouth.py
python3 train_gesture.py
python3 train_timbre.py
python3 train_chords.py
python3 make_fixture.py   # the runtime's own layer test models
```

Each script prints the accuracy of the float model and of the int8 one.
Then `cargo test` checks that the Rust runtime (`src/apps/neural.rs`)
gives exactly the Python reference's int8 outputs (`*.test.json`), and
that each app's audio features match `features.py` (`*.features.json`).

## Layers and quantisation

The networks only use what Neural-ART runs in hardware: 1-D convolution,
max pooling, flatten, global average pooling, dense layers and ReLU.
Quantisation is the TFLite / ONNX-QDQ scheme that ST Edge AI also uses:
- int8 activations, each with a scale and a zero point;
- int8 weights, with one scale per output channel;
- int32 biases and int32 accumulation;
- a float multiplier to requantise each output.

`pmxn.run_int8` is the reference implementation.

## On the device

The `.onnx` files are the float networks. ST Edge AI compiles them for
the NPU, using the same calibration data to quantise them:

```sh
stedgeai generate --model hum.onnx --target stm32n6 --st-neural-art
```

Its report gives each model's real NPU time and memory. The sim's
"NPU ~x%" figure is an estimate: the model's MACs at 25% of the 600 GOPS
peak. The `.pmxn` file is Portamax's own container for the int8 weights;
the sim runs it on the computer's CPU.
