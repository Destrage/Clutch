# Design: Setup checks

## Where the checks live
A pure function in `apps/estream` (`checks.rs`) takes the project snapshot and returns warnings
per source id. It runs on refresh, like the existing `DOUBLE_AUDIO` monitoring check (which moves
into it). No engine or real-time thread changes; nothing is saved.

Inputs per source: kind, settings (device, app, audio switch), muted. A source counts if it is in
the collection and not muted.

## Device identity
- Camera: `settings.device` (the device path).
- Audio input/output: `settings.device`, where "" is the default device. "" and the explicit id of
  the default device are not matched (the default can change); only equal strings count.
- Application audio: the executable in its settings.

## Desktop audio's one exclusion
Desktop audio on the default device leaves out one process: a captured game first, else an
Application audio app. Statically, with Desktop audio (default, unmuted) present and more than one
of {Application audio sources, Game/Window captures with audio on}, the Application audio sources
get the warning (capture audio wins the exclusion, so the apps are the ones heard twice).

## Toasts
After each executed command the app compares the set of (source, check) pairs before and after;
pairs that are new raise one toast. Loading a collection doesn't toast.

## Display
Warnings use the existing amber styling but a separate field from `problem` (which means "not
working"): rows get a "Check" badge with the text on hover, the Inspector shows the card, and the
mixer strip shows the dot. A source with both shows the problem.
