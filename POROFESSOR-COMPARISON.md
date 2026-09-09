# Porofessor feature comparison

Reviewed September 9, 2026 against Porofessor's [official app page](https://porofessor.gg/download) and [FAQ](https://porofessor.gg/faq). LuvvyLoL findings below come from this source tree, not a live macOS comparison.

| Area advertised by Porofessor | LuvvyLoL status |
| --- | --- |
| Draft bans and counterpick advice | Existing ban suggestions, matchup data and comfort-based pick recommendations. Provider coverage remains a dependency. |
| Rune, spell and item import | Existing support retained. This update adds a saved Flash key preference, including Swiftplay. |
| Matchup analysis | Existing damage composition, lane matching and strategy panels. Some advice is heuristic. |
| Player tags | Expanded by 24 evidence-based labels. Uses available local match history; exact Porofessor thresholds are not publicly documented on these pages. |
| Live overlay statistics | Existing live CS, gold estimates, spell timers and strategy information. This update enlarges the display and adds interactive build selection. |
| Jungle and inhibitor timers | Existing event-based objective timers and jungle heuristics are partial coverage. Full camp and inhibitor timer parity is not implemented. |
| Detailed match analysis | Existing KDA, farming, gold timelines, damage, vision and performance labels. Locally available timeline fields limit detail. |
| Champion meta tier list | Provider tier data supports recommendations; there is no complete standalone meta explorer comparable to the advertised feature. |
| Pro replay browsing | Not implemented. |
| Teammate finder | Not implemented. |
| TFT guidance and overlays | Not implemented. Outside the current League companion scope. |

Porofessor's FAQ says its statistics use normal and ranked 5v5 games from the last 30 days. The added LuvvyLoL labels follow that time window but only see matches supplied by the local client, so sample depth may differ. Labels expose their sample size and do not fabricate enough tags to fill every player card. Anonymous draft players cannot receive account-history labels until their identities are available.

## Remaining accuracy limitations

- No exact visual parity claim: this update follows the requested readability priorities, and still needs a rendered comparison on the user's monitor.
- Existing win probability, inferred premades, wave advice and jungle activity are estimates. They should not be mistaken for verified hidden game state.
- LP history tracks Solo/Duo. Flex LP is deliberately not combined or presented as a verified total; a separate Flex tracker remains future work.
- AP/AD auto detection selects among available paths. It is not a universal build generator for every unusual champion/item combination. Varus alternatives are curated templates, not claimed to be current statistical best builds.
- Enemy-specific advice stays separate from the core purchase sequence. It does not silently sell items or force an inventory rewrite.

## Priorities for another pass

1. Run macOS visual and interaction checks at actual monitor resolutions and display scaling.
2. Add verified separate Flex LP snapshots and queue filters if desired.
3. Expand champion-specific build templates using a reliable current source.
4. Improve timer coverage only where the data source provides verifiable events.

[Riot item metadata used for ID validation](https://ddragon.leagueoflegends.com/cdn/16.18.1/data/en_US/item.json).
