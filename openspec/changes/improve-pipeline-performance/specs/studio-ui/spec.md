## ADDED Requirements

### Requirement: Event-driven UI updates
The UI SHALL update when new canvas frames or engine events arrive, limited to the display
refresh rate, instead of polling on a fixed short interval. Slow housekeeping (chat, stream
details, health) SHALL run on its own timers. While the window is minimized or hidden, the UI
SHALL NOT wake for canvas frames.

#### Scenario: Minimized while streaming
- **WHEN** the window is minimized during a stream
- **THEN** the UI thread wakes only for events and slow timers, and chat and outputs keep working

#### Scenario: Restored
- **WHEN** the window is restored
- **THEN** the preview shows the current frame within one display refresh
