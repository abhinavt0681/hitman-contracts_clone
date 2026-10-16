# The Bjarkhov Bomb — atmosphere

Reference screenshots live in `data/reference/` (gitignored; from the Hitman fandom wiki page
"Bjarkhov's Base"). Every Contracts mission is set at night; this is the one where it snows
instead of rains, and the blizzard never stops.

## Exteriors
- Deep night. Sky and distance dissolve into a dark teal / blue-grey fog; silhouettes of the
  plane and hangars are barely readable beyond ~60 m.
- Constant blizzard: dense, fast, wind-driven snow streaks across the view.
- Light comes from sodium/white lamp posts throwing small pools on the snow, lit windows, and
  red beacons (plane tail fin, towers). Everything else is cold and desaturated.
- Snow ground reads pale blue-grey, never white.

## Interiors (mess hall, tunnels, submarine, guard rooms)
- Dim, green-tinted fluorescent tubes; grimy tiles and brick; pools of shadow.
- A few warm desk lamps (Bjarkhov's office, guard room) as accents.

## Mood
Cold, isolated, oppressive. Blood Money (same engine family) can inspire mechanics, but its
brighter daytime levels must not leak into the look here.

## Score

Jesper Kyd's original loops from the level bank, crossfaded by `src/music.rs`:

| State | Track |
|---|---|
| Unseen | `Music_Winter&Night` |
| Within ~10 m of the submarine hull | `Music_Submarine` |
| A guard's suspicion above 35% (held 8 s after it drops) | `Music_Suspense3` |
| Alarm | `Music_Sus1_High_Intense` |
| Mission failed | `Music_Death_Loop` |
| Mission complete | `Music_Main_Title_No_Intro` |

The calm and submarine loops pause while silent and resume in place; the others restart from
the top each time. Other cues in the bank (`Music_Atmos_Loop`, `Music_Sus2`,
`Music_Suspense1_Full`, `Music_Swat_Intense`) are spares for later states.

## Grade

The default is our own darker night (fog `NIGHT`, ambient 120, TonyMcMapface). `HITMAN_LOOK=faithful`
matches colours measured from gameplay footage of the original (snow ≈ 56,90,91; fog ≈ 55,85,90
sRGB): fog `srgb(0.20, 0.31, 0.33)`, ambient 700, lamps at 0.45× and no filmic tone curve
(`Tonemapping::Linear`), like the 2004 fixed-function renderer.

The level also contains the in-game map's floor plans (`C02-1__MAP_*`, drawn with
`HMC_IngameMap_Colors_01a`) laid over the real ground; they are skipped when spawning.
