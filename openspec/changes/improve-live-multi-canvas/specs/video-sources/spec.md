## ADDED Requirements

### Requirement: Warn About Sound Captured Twice
The mixer SHALL warn on a source's strip when its sound reaches the same canvas mix twice:
when two Application audio sources capture the same application, or when Desktop audio carries
an application that another source also captures because Desktop audio is leaving out a
different program. The warning SHALL name the programs and say how to fix it.

#### Scenario: Game capture, Discord and Desktop audio
- **WHEN** a game capture captures Apex's audio, an Application audio source captures Discord
  and Desktop audio is on
- **THEN** the Desktop audio strip warns that it also carries Discord because it leaves out
  Apex, and suggests muting Desktop audio or removing the Discord source

#### Scenario: Two sources on the same app
- **WHEN** two Application audio sources capturing Discord.exe are in one canvas's mix
- **THEN** both strips warn that Discord is captured twice
