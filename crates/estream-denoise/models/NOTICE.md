# DeepFilterNet3 low-latency model

`DeepFilterNet3_ll_onnx.tar.gz` is the pretrained DeepFilterNet3 low-latency model from the
DeepFilterNet project by Hendrik Schröter: <https://github.com/Rikorose/DeepFilterNet>
(`models/` in that repository). It is included unchanged and compiled into Clutch by
`src/lib.rs`.

## Licence status

DeepFilterNet's code is licensed MIT OR Apache-2.0 (see `../LICENSE-MIT` and
`../LICENSE-APACHE`). Upstream has not yet confirmed that the pretrained weights are covered by
the same terms. Clutch treats them as covered, and follows the open question in
<https://github.com/Rikorose/DeepFilterNet/issues/700>.

If the weights turn out to need different terms, this file will be updated or the model
replaced. Anyone redistributing Clutch should check that issue first.
