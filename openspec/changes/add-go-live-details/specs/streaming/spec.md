## ADDED Requirements

### Requirement: Go live window
Going live with a connected account (YouTube, Twitch or Kick) among the enabled destinations SHALL
open a window with that platform's stream details, unless the user chose to go live without asking.
Destinations without a connected account SHALL be listed as streamed with their key. Clutch SHALL
apply the details on every platform before it starts the video.

#### Scenario: Window on Go Live
- **WHEN** the user clicks Go Live with YouTube and a custom server enabled and YouTube connected
- **THEN** a window shows a YouTube section and lists the custom server as streamed with its key

#### Scenario: Details applied before the video
- **WHEN** the user clicks Go live in the window
- **THEN** Clutch applies each platform's details, and only then connects every destination

#### Scenario: A platform fails
- **WHEN** applying one platform's details fails
- **THEN** the window stays open with the error on that platform, Try again, and Go live without
  that platform

#### Scenario: Starting one destination
- **WHEN** the user clicks Start on one destination whose platform account is connected
- **THEN** the window opens for that platform only, and Go live there starts only that destination

#### Scenario: Double-click on Start
- **WHEN** the user clicks a destination's Start and then the same button again within 2 seconds
- **THEN** the destination keeps streaming

### Requirement: YouTube broadcast choice
The YouTube section SHALL list the channel's upcoming and ready broadcasts and SHALL always offer a
new broadcast, preselected when none exists. A new broadcast SHALL have a title, description,
privacy, category, made-for-kids, latency, auto-start and auto-stop. A chosen existing broadcast
SHALL be editable.

#### Scenario: None waiting
- **WHEN** the channel has no upcoming broadcast
- **THEN** New broadcast is selected, filled in with the last details used

#### Scenario: One waiting, new one anyway
- **WHEN** the channel has an upcoming broadcast and the user picks New broadcast
- **THEN** Clutch creates a new broadcast with the entered details and binds it to the
  destination's stream

#### Scenario: Existing broadcast on another key
- **WHEN** the user picks a broadcast bound to another of their streams
- **THEN** Clutch switches the destination to that stream's server and key before connecting

#### Scenario: Auto-start works
- **WHEN** Clutch creates a broadcast with auto-start on and then starts the video
- **THEN** YouTube starts the broadcast without Clutch reconnecting

### Requirement: Twitch and Kick channel details
The Twitch section SHALL show and edit the channel's title, category, tags and language. The Kick
section SHALL show and edit the title and category. Changes SHALL be saved to the channel when
going live. Where the platform reports no title or category (Kick while offline), the section
SHALL show the details used last time.

#### Scenario: Category from the platform's list
- **WHEN** the user types part of a category name
- **THEN** the matching categories are listed under the field, and only picking one of them
  changes the category

#### Scenario: New title on Twitch
- **WHEN** the user changes the Twitch title and category and clicks Go live
- **THEN** the Twitch channel shows the new title and category when the stream starts

#### Scenario: Missing permission
- **WHEN** the Twitch or Kick sign-in lacks the permission to edit the channel
- **THEN** that section asks the user to connect the account again, and the other platforms are
  not affected

### Requirement: Remembered details
Each destination SHALL remember its last details and pre-fill them. With "Go live without asking"
on, and for a hotkey, macro or remote start, Clutch SHALL use the remembered details without the
window and SHALL still set up YouTube before starting the video.

#### Scenario: Hotkey go-live
- **WHEN** the user starts the stream with a hotkey and YouTube has no broadcast waiting
- **THEN** Clutch creates one from the remembered details, binds it, starts the video, and says so
  in a message with a link to it

### Requirement: Edit details while live
While streaming, the live panel SHALL offer Edit details. It SHALL change the title, description,
category and (YouTube) privacy without reconnecting.

#### Scenario: Title change mid-stream
- **WHEN** the user edits the title while live on YouTube and Twitch and saves
- **THEN** both platforms show the new title and the stream keeps running

### Requirement: Unused broadcast cleanup
When the stream ends, a YouTube broadcast Clutch created for it SHALL be deleted only if YouTube
reports that it never went live.

#### Scenario: Stopped before live
- **WHEN** the user ends the stream before YouTube started the broadcast Clutch created
- **THEN** Clutch deletes that broadcast, so no upcoming stream stays on the channel

#### Scenario: Went live briefly
- **WHEN** the broadcast went live and the stream ended before Clutch's status check saw it
- **THEN** Clutch reads its status, sees it went live, and keeps it and its recording
