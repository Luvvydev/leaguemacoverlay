// Pure decision helpers shared by the main window and TAB overlay.
export interface RankSample {
  timestamp: number; lp: number; tier: string; rank: string;
  queue_type?: string; wins?: number | null; losses?: number | null;
}
export function rankValue(tier: string, rank: string, lp: number): number {
  const tiers: Record<string, number> = { IRON: 0, BRONZE: 400, SILVER: 800,
    GOLD: 1200, PLATINUM: 1600, EMERALD: 2000, DIAMOND: 2400,
    MASTER: 2800, GRANDMASTER: 2800, CHALLENGER: 2800 };
  const divisions: Record<string, number> = { IV: 0, III: 100, II: 200, I: 300,
    '4': 0, '3': 100, '2': 200, '1': 300 };
  const t = tier.toUpperCase();
  if (!(t in tiers) || !Number.isFinite(lp) || lp < 0) return NaN;
  if (tiers[t] === 2800) return tiers[t] + lp;
  const d = divisions[rank.toUpperCase()];
  return d === undefined || lp > 100 ? NaN : tiers[t] + d + lp;
}
export interface RecentMatch {
  game_id: number; queue_id: number; timestamp: number; duration_secs: number;
  champion_id: number; win: boolean; kills: number; deaths: number; assists: number;
  cs: number; vision_score: number; gold_earned: number; total_damage: number; position?: string;
}
export function dailySoloLp(samples: RankSample[], history: RecentMatch[], start: number): number | null {
  const matches = history.filter(m => m.queue_id === 420 && m.timestamp >= start && m.duration_secs > 300);
  if (!matches.length) return null;
  const sorted = samples.filter(s => s.queue_type === 'RANKED_SOLO_5x5'
    && s.timestamp >= start && Number.isFinite(rankValue(s.tier, s.rank, s.lp)))
    .sort((a, b) => a.timestamp - b.timestamp);
  const first = sorted[0], last = sorted[sorted.length - 1];
  if (!first || !last || first === last) return null;
  const firstGame = Math.min(...matches.map(m => m.timestamp));
  const lastEnd = Math.max(...matches.map(m => m.timestamp + m.duration_secs * 1000));
  if (first.timestamp > firstGame || last.timestamp < lastEnd) return null;
  if (first.wins == null || first.losses == null || last.wins == null || last.losses == null) return null;
  const wins = matches.filter(m => m.win).length;
  if (last.wins - first.wins !== wins || last.losses - first.losses !== matches.length - wins) return null;
  return rankValue(last.tier, last.rank, last.lp) - rankValue(first.tier, first.rank, first.lp);
}
export interface HistoryLabel { label: string; description: string; tone: 'gold' | 'green' | 'blue' | 'purple' | 'red' | 'orange' | 'muted'; }
export function historyLabels(history: RecentMatch[], now = Date.now()): HistoryLabel[] {
  const games = [...new Map(history.filter(m => [400, 420, 430, 440, 490].includes(m.queue_id)
    && m.game_id > 0 && m.duration_secs > 300 && m.gold_earned > 0 && m.timestamp <= now
    && m.timestamp >= now - 30 * 86400000).map(m => [m.game_id, m])).values()]
    .sort((a, b) => b.timestamp - a.timestamp);
  if (games.length < 5) return [];
  const n = games.length, minutes = games.reduce((s, g) => s + g.duration_secs / 60, 0);
  const sum = (key: 'cs' | 'vision_score' | 'gold_earned' | 'total_damage' | 'kills' | 'deaths' | 'assists') => games.reduce((s, g) => s + g[key], 0);
  const cs = sum('cs') / minutes, vision = sum('vision_score') / minutes;
  const damage = sum('total_damage') / minutes, gold = sum('gold_earned') / minutes;
  const kills = sum('kills') / n, deaths = sum('deaths') / n, assists = sum('assists') / n;
  const wr = games.filter(g => g.win).length / n;
  const roles = new Map<string, number>();
  for (const g of games) if (['TOP','JUNGLE','MIDDLE','BOTTOM','UTILITY'].includes(g.position || '')) roles.set(g.position!, (roles.get(g.position!) || 0) + 1);
  const main = [...roles].sort((a,b) => b[1]-a[1])[0];
  const farmer = main && main[0] !== 'UTILITY' && main[1] >= n * .6;
  const visionTarget = main?.[0] === 'UTILITY' ? 1.5 : main?.[0] === 'JUNGLE' ? .9 : .6;
  const labels: HistoryLabel[] = [];
  const add = (condition: boolean | undefined, label: string, evidence: string, tone: HistoryLabel['tone'] = 'blue') => {
    if (condition) labels.push({ label, description: `${evidence} Based on ${n} available normal/ranked games from the last 30 days.`, tone });
  };
  add(farmer && cs >= 8, 'CS GOD', `${cs.toFixed(1)} CS per minute.`, 'gold');
  add(farmer && cs >= 6.5 && cs < 8, 'STEADY FARMER', `${cs.toFixed(1)} CS per minute.`, 'green');
  add(farmer && cs < 4.5, 'LOW FARM', `${cs.toFixed(1)} CS per minute.`, 'orange');
  add(!!main && vision >= visionTarget * 1.5, 'VISION LEADER', `${vision.toFixed(2)} vision per minute; role target ${visionTarget}.`, 'green');
  add(!!main && vision > 0 && vision < visionTarget * .5, 'LOW VISION', `${vision.toFixed(2)} vision per minute; role target ${visionTarget}.`, 'orange');
  add(damage >= 850, 'DAMAGE LEADER', `${Math.round(damage)} champion damage per minute.`, 'red');
  add(damage >= 600 && damage < 850, 'STEADY DAMAGE', `${Math.round(damage)} champion damage per minute.`);
  add(gold >= 430, 'GOLD ENGINE', `${Math.round(gold)} gold earned per minute.`, 'gold');
  add(kills >= 8, 'KILL PRESSURE', `${kills.toFixed(1)} kills per game.`, 'red');
  add(assists >= 12, 'ASSIST MACHINE', `${assists.toFixed(1)} assists per game.`, 'green');
  add(deaths <= 3, 'HARD TO KILL', `${deaths.toFixed(1)} deaths per game.`, 'green');
  add(deaths >= 8, 'FREQUENT DEATHS', `${deaths.toFixed(1)} deaths per game.`, 'orange');
  add((kills + assists) / Math.max(1, deaths) >= 5, 'KDA LEADER', `${((kills+assists)/Math.max(1,deaths)).toFixed(1)} combined KDA.`, 'gold');
  add(n >= 10 && wr >= .7, 'WINNING FORM', `${Math.round(wr*100)}% recent win rate.`, 'green');
  add(n >= 10 && wr <= .3, 'LOSING FORM', `${Math.round(wr*100)}% recent win rate.`, 'orange');
  add(games.filter(g => g.deaths === 0).length >= 2, 'CLEAN GAMES', `${games.filter(g => g.deaths === 0).length} games without dying.`, 'green');
  add(games.filter(g => g.kills >= 10).length >= n * .4, 'DOUBLE DIGIT KILLS', 'At least 10 kills in 40% or more of recent games.', 'red');
  add(games.filter(g => g.assists >= 15).length >= n * .4, 'TEAM SUPPORT', 'At least 15 assists in 40% or more of recent games.');
  add(games.every(g => g.deaths <= 6), 'CONSISTENT SURVIVAL', 'No more than 6 deaths in any sampled game.', 'green');
  add(!!main && main[1] >= n * .8, 'ROLE REGULAR', `${main?.[1]} games in ${main?.[0]}.`, 'purple');
  add(roles.size >= 3, 'ROLE FLEX', `Played ${roles.size} known roles.`, 'purple');
  add(new Set(games.map(g => g.champion_id)).size >= 7, 'WIDE CHAMPION POOL', `Played ${new Set(games.map(g => g.champion_id)).size} champions.`, 'purple');
  const short = games.filter(g => g.duration_secs <= 1500), long = games.filter(g => g.duration_secs >= 2100);
  add(short.length >= 5 && short.filter(g => g.win).length / short.length >= .7, 'SHORT GAME WINNER', `Won ${short.filter(g=>g.win).length}/${short.length} games lasting at most 25 minutes.`, 'green');
  add(long.length >= 5 && long.filter(g => g.win).length / long.length >= .7, 'LONG GAME WINNER', `Won ${long.filter(g=>g.win).length}/${long.length} games lasting at least 35 minutes.`, 'green');
  return labels;
}

export interface PathItem {
  gold: number; purchasable: boolean; from: number[]; into: number[];
  stats: { abilityPower: number; attackDamage: number; lethality: number };
}
export interface BuildPath { id: string; label: string; core: number[]; boots: number[]; style: 'AP' | 'AD' | 'Mixed'; }
export function availablePaths(champion: number, core: number[], boots: number[], alternatives: number[][],
  item: (id: number) => PathItem | null): BuildPath[] {
  const style = (ids: number[]): BuildPath['style'] => {
    const ap = ids.reduce((s,id) => s + (item(id)?.stats.abilityPower || 0),0);
    const ad = ids.reduce((s,id) => s + (item(id)?.stats.attackDamage || 0),0);
    return ap >= 100 && ap > ad * 2 ? 'AP' : ad >= 60 && ad > ap ? 'AD' : 'Mixed';
  };
  const paths: BuildPath[] = [{ id:'recommended', label:`Recommended (${style(core)})`, core, boots, style:style(core) }];
  for (const ids of alternatives) {
    if (!ids.length || ids.join('-') === core.join('-') || !ids.every(id => item(id)?.purchasable)) continue;
    const id = `items-${ids.join('-')}`;
    if (paths.some(p => p.id === id)) continue;
    paths.push({ id, label:`${style(ids)} option ${paths.length}`, core:ids, boots, style:style(ids) });
  }
  // Explicit off-meta Varus choices: curated templates, not claimed win-rate leaders.
  if (champion === 110) {
    const variants: BuildPath[] = [
      {id:'varus-ap',label:'Varus AP',style:'AP',core:[3115,3089,3135,4645,3157],boots:[3020]},
      {id:'varus-onhit',label:'Varus AD on hit',style:'AD',core:[3153,3124,3085,3036,3026],boots:[3006]},
      {id:'varus-lethality',label:'Varus AD lethality',style:'AD',core:[3142,6692,6694,3814,3026],boots:[3158]},
    ];
    for (const p of variants) if ([...p.core,...p.boots].every(id => item(id)?.purchasable)) paths.push(p);
  }
  return paths;
}
const SITUATIONAL = new Set([3157,3102,3026,3814,3139,3140,3124,3111,3047]);
export function choosePath(paths: BuildPath[], selection: string, owned: number[], item: (id:number)=>PathItem|null): BuildPath | undefined {
  if (selection !== 'auto') return paths.find(p=>p.id===selection) || paths[0];
  // Ignore components, boots, mixed-stat items, and common defensive pivots.
  const signals = owned.filter(id => {
    const data = item(id);
    return data && data.gold >= 2200 && !SITUATIONAL.has(id)
      && (data.stats.abilityPower >= 60 || data.stats.attackDamage >= 35);
  });
  if (!signals.length) return paths[0];
  const score = (p:BuildPath) => signals.reduce((sum,id) => sum + (p.core.includes(id) ? 3 : 0),0);
  if (!paths.length) return undefined;
  const base = score(paths[0]);
  const ranked = paths.map(p=>({p,score:score(p)})).sort((a,b)=>b.score-a.score);
  // An exact completed-item match must improve on the current default.
  const best = ranked[0];
  const tiedStyles = new Set(ranked.filter(r => r.score === best.score).map(r => r.p.style));
  return best.score > base && tiedStyles.size === 1 ? best.p : paths[0];
}
export function componentCredit(target:number, inventory:number[], item:(id:number)=>PathItem|null):number {
  const available = [...inventory];
  const visit = (id:number, seen:Set<number>):number => {
    if (seen.has(id)) return 0;
    const index = available.indexOf(id);
    if (index >= 0) { available.splice(index,1); return item(id)?.gold || 0; }
    const next = new Set(seen); next.add(id);
    return (item(id)?.from || []).reduce((sum,child)=>sum+visit(child,next),0);
  };
  return Math.min(item(target)?.gold || 0, visit(target,new Set()));
}
