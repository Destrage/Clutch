## ADDED Requirements

### Requirement: Readable Text
Every text color in the studio's dark and light themes SHALL have a contrast ratio of at least
4.5:1 against every surface color of the same theme, including secondary and hint text. No text
SHALL use a fixed font size outside the theme's size tokens.

#### Scenario: Hint text on a raised control
- **WHEN** hint text is drawn on the most raised surface of the dark theme
- **THEN** its contrast ratio is at least 4.5:1

#### Scenario: Light theme
- **WHEN** the user switches to the light theme
- **THEN** secondary and hint text also reach at least 4.5:1 on every light surface

### Requirement: Text Size Setting
Settings → Appearance SHALL offer a text size of Small (90%), Default, Large (115%) or Larger
(130%). The choice SHALL apply to all studio text at once, without a restart, and SHALL be
remembered. Controls that hold text SHALL grow with it, so no text is clipped at any size.

#### Scenario: Larger text
- **WHEN** the user picks Larger
- **THEN** all text in the studio is drawn 30% larger right away, labels in the deck, sidebar,
  inspector and settings are not cut off, and the choice is still set after a restart

#### Scenario: Unknown value in settings
- **WHEN** the settings file holds a text size the version doesn't know
- **THEN** Default is used and the other settings load normally
