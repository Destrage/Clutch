## ADDED Requirements

### Requirement: Copy the sign-in link
When connecting an account, users SHALL be able to copy the sign-in link instead of having it
opened in the default browser, so they can sign in from any browser on the same PC.

#### Scenario: Copy before connecting
- **WHEN** the user clicks Copy link on a YouTube or Kick destination that isn't connected
- **THEN** Clutch starts the sign-in without opening a browser, puts the sign-in link on the
  clipboard and waits for the sign-in as it does after Connect

#### Scenario: Copy while waiting
- **WHEN** a connect is waiting for the user to sign in or enter a code
- **THEN** Copy link puts the page being waited on on the clipboard

#### Scenario: Signing in from another browser
- **WHEN** the user opens the copied link in a browser other than the default one and signs in
- **THEN** the account connects as it does from the default browser
