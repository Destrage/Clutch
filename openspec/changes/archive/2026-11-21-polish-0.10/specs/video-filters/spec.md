## ADDED Requirements

### Requirement: Filter Values as Percentages
Video filter settings that are fractions SHALL be shown and entered as percentages, while the
stored values stay fractions.

#### Scenario: Opacity
- **WHEN** a Color correction filter has opacity 1.0
- **THEN** its Opacity slider reads "100%", and setting it to 40% stores 0.4
