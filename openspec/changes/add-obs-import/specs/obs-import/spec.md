## ADDED Requirements

### Requirement: Import OBS Scene Collections
The system SHALL import an OBS Studio scene collection as a new setup, found in the default
OBS config folder or chosen as a file. It SHALL map scenes, scene items with their transforms,
groups, nested scenes, the supported source types, audio settings and the supported filters.
It SHALL never overwrite an existing setup.

#### Scenario: Import a gaming collection
- **WHEN** the user imports an OBS collection named "Apex" with a game capture, a webcam, an image overlay and a browser alert box
- **THEN** a new setup "Apex" is created with the same scenes, and each item in the same place and size

#### Scenario: Name already taken
- **WHEN** a setup named "Apex" already exists and the user imports a collection named "Apex"
- **THEN** the new setup is named "Apex (2)" and the existing one is unchanged

#### Scenario: Blocked while live
- **WHEN** the user starts an import while streaming or recording
- **THEN** the import does not start and the user is told to stop first

### Requirement: Import OBS Profiles
The system SHALL optionally import an OBS profile's canvas size and fps, recording folder and
format, encoder and bitrate, and stream service and server. The stream key SHALL be imported
only when the user explicitly asks for it.

#### Scenario: Profile without the key
- **WHEN** the user imports a profile and leaves "Import my stream key" unticked
- **THEN** the service and server are set up and Clutch asks for the key when the user first goes live

### Requirement: Import Report
After an import the system SHALL show what was imported and what was skipped or changed, each
with a reason. A skipped source SHALL become a placeholder in its place in the scene.

#### Scenario: Plugin source
- **WHEN** the collection contains a source from a third-party OBS plugin
- **THEN** the report lists it as skipped ("plugin source not available") and the scene shows a placeholder where it was
