## ADDED Requirements

### Requirement: Output Scaling
Recording, the replay buffer and each stream destination SHALL each be able to encode at the
canvas size, 720p or 480p, and at the canvas frame rate or half of it, without changing the
canvas, its scenes or the preview. Scaling SHALL run on the GPU.

#### Scenario: 720p30 stream from a 1080p60 canvas
- **WHEN** a destination is set to 720p at half fps on a 1920 × 1080, 60 fps canvas
- **THEN** the stream is encoded at 1280 × 720 and 30 fps while the recording and preview stay at 1080p60

### Requirement: Low-End PC Profile
The system SHALL offer a Low-end PC profile, lighter than Gaming, that encodes outputs at 720p
and half the canvas fps with the fastest encoder preset, uses a 30 s replay buffer, and draws
the preview at 15 fps while live or recording. On applying it, the system SHALL list heavier
features in the current setup as optional changes that are off by default.

#### Scenario: Suggestions not applied by default
- **WHEN** the user applies Low-end PC on a setup that uses strong noise suppression
- **THEN** the confirmation offers to switch it to standard, unticked, and noise suppression is unchanged unless the user ticks it

### Requirement: Low-End PC Recommendation
Optimize for this PC SHALL recommend Low-end PC for PCs without a hardware encoder and with 4
cores or fewer, with under 8 GB of memory, or with only an integrated GPU that cannot encode
HEVC. The lighter-profile action on overload alerts SHALL step from Gaming to Low-end PC.

#### Scenario: Old laptop
- **WHEN** Optimize for this PC runs on a 4-core laptop with 8 GB and only an integrated GPU that has no HEVC encoder
- **THEN** it recommends Low-end PC and lists each setting it will change, before and after

#### Scenario: Stepping down from Gaming
- **WHEN** an encoder-overload alert appears during a game while the Gaming profile is active
- **THEN** its "Use a lighter profile" action switches to Low-end PC after confirmation

### Requirement: Simulated Hardware
For development and testing, the system SHALL read `CLUTCH_SIMULATE_HW` and, when set, report
the given cores, memory and GPU type from the hardware probe and hide hardware encoders when
asked. The log and the Performance page SHALL say that the hardware is simulated.

#### Scenario: Simulate a PC without hardware encoders
- **WHEN** Clutch starts with `CLUTCH_SIMULATE_HW=cores=2,memory=4,no-hw-encoders`
- **THEN** the Performance page shows "Simulated hardware", no hardware encoder is offered, and Optimize recommends Low-end PC
