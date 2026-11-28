# Tasks: add-gpu-selection

## 1. Adapter selection
- [ ] 1.1 Enumerate GPUs with name, vendor/device id, which displays they drive, and their encoders
- [ ] 1.2 Selection by setting, with fallback to Automatic and a one-time toast
- [ ] 1.3 Automatic rules and their logged reason, with a test per case
- [ ] 1.4 Capture interop and the UI/engine device pair on the chosen adapter

## 2. Encoder on another GPU
- [ ] 2.1 Encoder list per GPU
- [ ] 2.2 Cross-adapter shared texture copy at output size, once per output frame
- [ ] 2.3 Optimize for this PC recommends the integrated encoder on hybrid laptops

## 3. UI
- [ ] 3.1 Graphics card row on Settings → Performance, restart note, display warning, reason text

## 4. Check
- [ ] 4.1 Tests; hybrid laptop (GTX 1650 Ti + Intel UHD): each GPU, each encoder, game fps with and without Clutch live
