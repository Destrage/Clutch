## ADDED Requirements

### Requirement: Graphics Card Choice
Settings → Performance SHALL offer the graphics card Clutch composites on: Automatic (the
default) or any GPU in the PC by name. The choice SHALL apply after a restart. Automatic SHALL
show which GPU it picked and why. When the chosen GPU doesn't drive the main display, the
setting SHALL warn that screen capture copies frames between GPUs. When the chosen GPU is
missing at startup, Clutch SHALL use Automatic and say so once.

#### Scenario: Pick the integrated GPU
- **WHEN** the user on a hybrid laptop picks the integrated GPU and restarts
- **THEN** compositing and display capture run on the integrated GPU and the Performance page names it

#### Scenario: GPU removed
- **WHEN** the chosen GPU is an external GPU that is unplugged at startup
- **THEN** Clutch starts on the Automatic choice and shows a toast saying the chosen card wasn't found

### Requirement: Encoding on Another GPU
The encoder list SHALL include hardware encoders on other GPUs than the one Clutch composites
on, named with their GPU. Using one SHALL copy each output frame to that GPU once, at output
size. On a hybrid laptop with an integrated encoder, Optimize for this PC SHALL recommend it.

#### Scenario: QuickSync while compositing on NVIDIA
- **WHEN** Clutch composites on the discrete NVIDIA GPU and the user picks "QuickSync (Intel UHD 630)" for the stream
- **THEN** the stream is encoded by the Intel GPU and the NVIDIA GPU's encoder is not used
