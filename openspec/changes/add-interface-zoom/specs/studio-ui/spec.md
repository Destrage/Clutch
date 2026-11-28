## ADDED Requirements

### Requirement: Interface Zoom
Settings → Appearance SHALL offer an interface zoom of 90%, 100%, 110%, 125%, 150%, 175% or
200% that scales all of the studio's text, icons, controls and spacing, on top of the OS
display scaling. It SHALL apply at once without a restart and SHALL be remembered.
Ctrl/Cmd + `=`, `-` and `0` SHALL step and reset it. A level whose minimum window size does not
fit the current monitor SHALL be unavailable there.

#### Scenario: Zoom in
- **WHEN** the user presses Ctrl + `=` at 100%
- **THEN** the whole interface is drawn at 110% right away, a toast shows "Zoom 110%", and the level is still set after a restart

#### Scenario: Too large for the screen
- **WHEN** the window is on a 1366 × 768 monitor at 100% OS scaling
- **THEN** 150% and larger zoom levels are greyed out with "Too large for this screen"

#### Scenario: Moving to a smaller monitor
- **WHEN** the user has 175% zoom on a 4K monitor and drags the window to a monitor where 175% doesn't fit
- **THEN** the zoom steps down to the largest level that fits, and returns to 175% when the window goes back

## MODIFIED Requirements

### Requirement: Text Size Setting
Settings → Appearance SHALL offer a text size of Small (90%), Default, Large (115%), Larger
(130%) or Largest (150%). The choice SHALL apply to all studio text at once, without a restart,
and SHALL be remembered. It SHALL multiply with the interface zoom. Controls that hold text
SHALL grow with it, so no text is clipped at any size.

#### Scenario: Larger text
- **WHEN** the user picks Larger
- **THEN** all text in the studio is drawn 30% larger right away, labels in the deck, sidebar,
  inspector and settings are not cut off, and the choice is still set after a restart

#### Scenario: Largest text with zoom
- **WHEN** the user picks Largest text and 150% zoom
- **THEN** text is drawn at 225% of its default size and no label in the deck, sidebar, inspector or settings is cut off

#### Scenario: Unknown value in settings
- **WHEN** the settings file holds a text size the version doesn't know
- **THEN** Default is used and the other settings load normally
