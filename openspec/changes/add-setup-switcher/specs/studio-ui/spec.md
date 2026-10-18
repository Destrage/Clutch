## ADDED Requirements

### Requirement: Setups
The studio SHALL let users keep several setups (scene collections) and manage them from a menu on
the setup name in the top bar: switch, create (through the first-run flow), rename, duplicate and
delete.

#### Scenario: Switching setups
- **WHEN** the user picks another setup from the menu while not streaming or recording
- **THEN** the current setup is saved and the chosen one opens

#### Scenario: Switching while live
- **WHEN** the user picks another setup while streaming or recording
- **THEN** nothing changes and a message asks them to stop first

#### Scenario: New setup
- **WHEN** the user chooses New setup and enters a name
- **THEN** the first-run setup opens for that setup, and Cancel returns to the previous setup

#### Scenario: Deleting a setup
- **WHEN** the user deletes the open setup and confirms
- **THEN** its file is removed and another setup opens
- **AND** Delete is not offered when only one setup exists
