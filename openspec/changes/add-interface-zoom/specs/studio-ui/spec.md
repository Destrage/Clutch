## ADDED Requirements

### Requirement: Interface Zoom
Settings → Appearance SHALL offer an interface zoom of 100%, 125%, 150%, 175% or 200% that
scales the whole studio (text, icons, controls, spacing, popups and dialogs) on top of the
operating system's display scaling. It SHALL apply without a restart, SHALL be remembered, and
SHALL be adjustable with Ctrl+Shift+=, Ctrl+Shift+- and Ctrl+Shift+0. A level that would make
the studio smaller than its minimum layout on the current monitor SHALL NOT be selectable.

#### Scenario: Zoom in
- **WHEN** the user picks 150%
- **THEN** every control, icon and text in the studio is drawn 50% larger right away, and
  150% is still set after a restart

#### Scenario: Preview keeps real pixels
- **WHEN** the zoom is 200% and the user chooses the preview's 100%
- **THEN** one canvas pixel is drawn on one screen pixel

#### Scenario: Level doesn't fit
- **WHEN** the window is on a 1920×1080 monitor at 100% display scaling
- **THEN** 175% and 200% are disabled and say the screen is too small for them

#### Scenario: Moving to a smaller monitor
- **WHEN** the window moves from a 4K monitor at 200% zoom to a monitor where 200% doesn't fit
- **THEN** the zoom steps down to the largest level that fits and a toast says so

### Requirement: Narrow Window Layout
The studio SHALL stay usable at its minimum logical size (980 × 620) at every interface zoom
and text size. When the window is too narrow for the top bar's labels, the top bar SHALL show
icons with tooltips instead of labels, keeping Go Live labelled. A dialog taller than the
window SHALL be limited to the window's height and its content SHALL scroll, with a scroll
indicator.

#### Scenario: Go Live stays reachable
- **WHEN** the window is 1000 logical pixels wide (for example 150% zoom on a 1500-pixel-wide window)
- **THEN** every top-bar control is visible, Studio mode, Record and search show as icons, and Go Live keeps its label

#### Scenario: Tall dialog in a short window
- **WHEN** the first-run wizard opens in a window shorter than the wizard
- **THEN** the wizard fits the window with a margin, a scroll indicator shows, and the wheel scrolls to Get started

## MODIFIED Requirements

### Requirement: Text Size Setting
Settings → Appearance SHALL offer a text size of Small (90%), Default, Large (115%), Larger
(130%) or Largest (150%). The choice SHALL apply to all studio text at once, without a restart,
and SHALL be remembered. Controls that hold text SHALL grow with it, so no text is clipped at
any size. Text size SHALL multiply with the interface zoom.

#### Scenario: Larger text
- **WHEN** the user picks Larger
- **THEN** all text in the studio is drawn 30% larger right away, labels in the deck, sidebar,
  inspector and settings are not cut off, and the choice is still set after a restart

#### Scenario: Largest with zoom
- **WHEN** the user picks Largest with a 200% interface zoom
- **THEN** text is drawn at three times its default size and no label in the deck, sidebar,
  inspector or settings is cut off

#### Scenario: Unknown value in settings
- **WHEN** the settings file holds a text size the version doesn't know
- **THEN** Default is used and the other settings load normally
