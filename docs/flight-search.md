# Flight search and fare monitoring from São Paulo

Research notes; nothing in this repo runs any of it yet. Sections 1–4 cover what exists
for searching and watching fares programmatically as of November 2026. Section 5 is a
design for a fare monitor that could run here as a systemd timer. Section 6 is a worked
example: São Paulo ⇄ Brasília in January 2027.

The **Checked** column and the notes in brackets say how each claim was established:
**ran** means exercised from a São Paulo IP (on 2026-11-09 unless another date is given),
**docs** means the vendor's own documentation, and **secondary** means a third-party
source only, so treat it as unverified.

---

## 1. What to use

| # | Option | What it gives | Cost | Status | Checked |
|---|---|---|---|---|---|
| 1 | Google Flights deals page (§3.1) | 30 deals from São Paulo, each with the current price, Google's usual price and % below usual | free | beta; undocumented page structure | ran, again 2026-11-11 |
| 2 | Google Explore RPC (§3.2) | about 70 priced destinations per call, by month and trip length | free | undocumented | ran |
| 3 | [Kiwi.com MCP](https://www.kiwi.com/en/pages/mcp), `https://mcp.kiwi.com` | search by city, region or "anywhere"; flexible dates and nights; BRL; LATAM, GOL and Azul from GRU, CGH and VCP | free, no key | official | ran |
| 4 | [fli](https://github.com/punitarani/fli) (PyPI `flights`): CLI, Python, `fli-mcp` | Google Flights per-route search and date sweeps | free | active; unreliable on thin routes | ran, again 2026-11-11 |
| 5 | [Skiplagged MCP](https://skiplagged.github.io/mcp), `https://mcp.skiplagged.com/mcp` | "anywhere" search with BRL display | free, no key | official; prices are converted from USD and came out about 40% high on GRU–CPT | ran |
| 6 | [SerpApi](https://serpapi.com/pricing) `google_travel_explore` and `google_flights` | documented Explore and per-route price insights/history; not a verified replacement for Deals' usual-price signal | 250 searches/month free, then $25 per 1k or $75 per 5k | active; not exercised here | docs |
| 7 | [seats.aero](https://seats.aero) Pro API | award space for GOL Smiles and Azul Fidelidade, not LATAM Pass (§4.3) | about $12.99/month (secondary) | active; the API is not offered in every country | docs |
| 8 | [Ignav](https://ignav.com/pricing), [SearchApi](https://www.searchapi.io/pricing), Apify actors | paid Google Flights scrapers, as fallbacks | Ignav $2 per 1k after 1k free; SearchApi 100 free, then $40 per 10k | active | secondary |
| 9 | [Travelpayouts / Aviasales Data API](https://support.travelpayouts.com/hc/en-us/articles/203956163-Aviasales-Data-API) | cached prices from other users' searches, including origin-only queries | free with an affiliate token | active; depth for Brazil unverified | docs |

Gone, or closed to individuals:

- **Amadeus Self-Service** shut down on 2026-07-17 and its keys stopped working [docs:
  the banner on developers.amadeus.com;
  [PhocusWire](https://www.phocuswire.com/amadeus-shut-down-self-service-apis-portal-developers)].
  Its enterprise APIs remain, but not for individuals.
- **Kiwi Tequila** has been invite-only for new partners since about May 2024
  [secondary]. The Kiwi MCP exposes similar search parameters.
- **Skyscanner's API** is partner-only, with a minimum of roughly 100k monthly users
  [secondary].
- **Going** only covers departures from the US [docs: its FAQ].
- **Duffel** is a booking API with no "anywhere" search. Without bookings, every search
  is billed at $0.005 [[pricing](https://duffel.com/pricing)].
- **fast-flights** 3.1.0 worked for GRU→LIS but crashed in its parser on GRU→CPT [ran].

---

## 2. From an agent

Both hosted MCP servers worked from Brazil without an account:

```
claude mcp add --transport http kiwi-com-flight-search https://mcp.kiwi.com
claude mcp add --transport http skiplagged https://mcp.skiplagged.com/mcp
```

For omp, register the same URLs as HTTP MCP servers. fli's server runs over stdio. No
host has `uv`, and `flights` 0.9.0 imports `click` without declaring it, so:

```
nix shell nixpkgs#uv -c uvx --with click --from 'flights[mcp]' fli-mcp
```

[ran 2026-11-11: that resolves `flights` 0.9.0 with the `fli`, `fli-mcp` and
`fli-mcp-http` entry points, and `fli --help` runs the same way.]

Kiwi's `search-flight` tool takes `flyFrom`/`flyTo` as a city, an airport, a region or
`anywhere`; dates as dd/mm/yyyy, with ± flexible days or ranges;
`nights_in_dst_from`/`nights_in_dst_to`; `one_for_city`; price, stopover and airline
filters; and a currency. `anywhere` returns the cheapest results first, which from São
Paulo means only domestic flights, so ask per region instead. For March 2027 and 7–14
nights, "Africa" gave GRU–CPT R$4,650 (LATAM nonstop) and GRU–JNB R$4,272; "Europe" gave
VCP–LIS R$3,914 (Azul nonstop) and GRU–MAD R$4,080 [ran].

fli is the tool for confirming a fare on exact dates. From its main branch,
`search_dates` for GRU⇄CPT, 14-day trips starting 1–20 March, took 1.6 s and found
R$4,283 (4–18 March). A plain round-trip search on that route returned null-priced
itineraries in 0.9.0 or no results on main, while GRU⇄LIS worked [ran].

Consumer features, none of them schedulable: Google Flights' "Flight Deals" (beta) works
with `gl=BR` and takes natural-language prompts in the browser; Kiwi ships ChatGPT,
Claude and Gemini connectors on the same MCP; Kayak has "Ask AI" [secondary].

---

## 3. Google Flights, directly

Everything in this section is undocumented and changes without notice: according to
fli's README, since 2026-08 the search and calendar RPCs return empty responses to plain
HTTP clients that lack a botguard header (`x-goog-batchexecute-bgr`). `robots.txt`
disallows `/travel/flights/search` and `/travel/flights/booking` but not
`/travel/flights/deals` or `/travel/explore`, and Google's terms forbid abusive automated
access, so keep it to a few requests per run. The deals, Explore and calendar prices are
cached minimums: confirm on a live search before buying.

### 3.1 Deals page

`GET https://www.google.com/travel/flights/deals?hl=en&gl=BR&curr=BRL` works with plain
curl: no cookies, no JavaScript [ran, again 2026-11-11]. The deals are in the
`AF_initDataCallback` block with key `ds:1`, as a list at `data[3][9]`:

| index | field |
|---|---|
| `[1]`, `[2]` | departure and return dates, as `[y, m, d]` |
| `[3][0][1]` | current price |
| `[4][0][1]` | usual price |
| `[5]` | % below usual |
| `[7]` | duration, minutes |
| `[8]` | stops |
| `[10]`, `[11]` | airline code and name |
| `[13][0]` | destination city |

The airports appear as three-letter strings inside the record. This printed all 30 deals
on 2026-11-11:

```python
import json, re, sys

html = open(sys.argv[1]).read()
blocks = dict(re.findall(
    r"AF_initDataCallback\(\{key: '(ds:\d+)'.*?data:(.*?), sideChannel: \{\}\}\);", html, re.S))
for d in json.loads(blocks["ds:1"])[3][9]:
    codes = re.findall(r'"([A-Z]{3})"', json.dumps(d))
    print(f"{d[1][0]}-{d[1][1]:02d}-{d[1][2]:02d} to {d[2][1]:02d}-{d[2][2]:02d}",
          f"R${d[3][0][1]} usual R${d[4][0][1]} -{d[5]}%",
          f"stops={d[8]} {d[10]} {d[13][0]} {codes[:2]}")
```

- **The origin comes from the requester's IP**: São Paulo here, mixing GRU, CGH and VCP.
- **The list stops at 30, ranked by % below usual**, so cheap domestic routes crowd out
  international ones.
- **Google's definition** [docs: <https://support.google.com/travel/answer/16497283>]: a
  deal is at least 20% below the typical price, and the typical price is the median of
  the route's cheapest prices over the past 12 months, adjusted for season, trip length
  and cabin.
- **Natural-language queries** (`?q=Flights from Sao Paulo to Africa or Europe`) are
  rendered client-side: they work in headless Chromium, not with curl.

Seen: on 2026-11-09, GRU–CPT 11–17 December at R$4,284 against a usual R$8,559 (−50%,
LATAM nonstop); on 2026-11-11, GRU–CPT 25 February–4 March at R$3,719 against R$7,847
(−53%, LATAM, one stop).

### 3.2 Explore

`https://www.google.com/travel/explore?hl=en&gl=BR&curr=BRL` defaults to São Paulo to
anywhere. The page's own `GetExploreDestinations` request carries
`x-goog-batchexecute-bgr`, but replaying it with `fetch()` from inside the page without
that header still returned full data [ran].

- **Origin:** `[["/m/022pfm",4]]` (São Paulo, the city) or `[["GRU",0]]`. About 70 priced
  destinations come back per call, each with dates, price, airline, stops, duration and
  IATA code.
- **Dates:** fixed dates in the legs are ignored. The flexible month and trip length
  come from the `tfs` URL protobuf, field 16: `{1: duration (1 weekend, 2 one week,
  3 two weeks), 2: month}`.
- **Currency:** USD with a city origin returned nothing; keep BRL.

### 3.3 Calendar: every departure date × trip length in one call

This is what the price map in §6 came from [ran 2026-11-09 and 2026-11-11]. Run it in a
page on `https://www.google.com/travel/flights` (the devtools console, or a headless
browser):

```js
const leg = (from, to, date) =>
  [[[[from, 0]]], [[[to, 0]]], null, 0, null, null, date, null, null, null, null, null, null, null, 3];
const inner = [
  null,
  [null, null, 1, null, [], 1, [1, 0, 0, 0], null, null, null, null, null, null,
    [leg("CGH", "BSB", "2027-01-09"), leg("BSB", "CGH", "2027-01-16")],
    null, null, null, 1],
  ["2027-01-01", "2027-01-31"], // departure dates to cover
  null,
  [5, 10], // trip lengths to cover, in days
];
const res = await fetch(
  "/_/FlightsFrontendUi/data/travel.frontend.flights.FlightsFrontendService/GetCalendarGraph" +
    "?hl=en&gl=BR&soc-app=162&soc-platform=1&soc-device=1&rt=c",
  {
    method: "POST",
    headers: {
      "content-type": "application/x-www-form-urlencoded;charset=utf-8",
      "x-same-domain": "1",
      "x-goog-ext-259736195-jspb": '["en-US","BR","BRL",1,null,[180],null,null,7,[]]',
    },
    body: "f.req=" + encodeURIComponent(JSON.stringify([null, JSON.stringify(inner)])) + "&",
  },
);
const rows = [];
for (const line of (await res.text()).split("\n")) {
  if (!line.trim().startsWith("[")) continue;
  let items;
  try { items = JSON.parse(line); } catch { continue; }
  for (const item of items) {
    if (item[0] !== "wrb.fr" || !item[2]) continue;
    for (const r of JSON.parse(item[2])[1] ?? []) {
      if (Array.isArray(r) && typeof r[0] === "string") rows.push({ depart: r[0], back: r[1], brl: r[2]?.[0]?.[1] });
    }
  }
}
console.table(rows);
```

- **Trip type:** `inner[1][2]` is `1` for round trip. The two leg dates
  only seed the search; the range and the trip lengths decide what comes back.
- **One way:** use `2` instead of `1`, a single leg, and stop after the date range:
  `[null, settings, [first, last]]`.
- **Currency:** the third element of the `x-goog-ext-259736195-jspb` header.
- **The response** is Google's batchexecute format: a `)]}'` line, then length-prefixed
  JSON chunks. The `wrb.fr` entry carries the payload as a JSON string, and the
  payload's second element lists `[depart, back, [[null, price], token], 1]`.
- **The page's "Date grid" view** calls `GetCalendarGrid` with the same settings,
  followed by `[firstDepart, lastDepart], [firstBack, lastBack]`. That was seen in the
  page's own request, not replayed.
- **Calendar values are cached** and a live search can differ: on 2026-11-09 the
  calendar said R$938 for CGH 6–13 January, and the live search found R$907.

### 3.4 Search pages

A [round-trip search](https://www.google.com/travel/flights?q=Flights%20to%20BSB%20from%20CGH%20on%202027-01-09%20through%202027-01-16&curr=BRL&hl=en&gl=BR)
uses `q=Flights to BSB from CGH on 2027-01-09 through 2027-01-16`; a one-way search uses
`q=One way flights to BSB from CGH on 2027-01-09`. URL-encode `q` and keep `curr=BRL`.
Each result's `aria-label` carries its advertised price and flight details:
"From 737 Brazilian reals round trip total. Nonstop flight with LATAM. Operated by Latam
Airlines Brasil. Leaves São Paulo/Congonhas–… at 7:15 AM on Saturday, January 9 and
arrives at … at 9:00 AM on Saturday, January 9. Total duration 1 hr 45 min."

The round-trip list prices each outbound with its cheapest return. Clicking an outbound
moves to `/travel/flights/search?tfs=…` (the path `robots.txt` disallows) and lists that
outbound's returns with round-trip totals.

---

## 4. Deals, alerts and miles

### 4.1 Alerts, no code

- **Google Flights tracking** [docs: <https://support.google.com/travel/answer/6235879>]:
  per itinerary, plus an "Any dates" toggle that emails when a route's price drops
  significantly. It needs a Google account and covers one route per tracker, so no
  "anywhere".
- **Kayak** (kayak.com.br) and **Skyscanner** alerts are route or date based
  [secondary].
- **Hopper** predicts prices in its app and powers Nubank's travel portal in Brazil
  [secondary]. No API.

### 4.2 Feeds for São Paulo departures

| Source | Relevance | Machine-readable |
|---|---|---|
| Melhores Destinos | the main Brazilian deal blog: international from São Paulo, plus miles promotions | the RSS feed `https://www.melhoresdestinos.com.br/feed` and the public Telegram preview `https://t.me/s/melhoresdestinos` both fetched without login [ran] |
| Passagens Imperdíveis | strong for Brazil | `/feed/` returned 404; Telegram and Instagram channels exist [secondary] |
| Pontos pra Voar | miles alerts | RSS behind Cloudflare, 403 to curl |
| Secret Flying | São Paulo category, error fares | `/feed/` behind Cloudflare, 403 to curl |
| Going | none: US departures only [docs] | — |
| Dollar Flight Club | claims South American airports; GRU unverified [secondary] | — |

### 4.3 Miles

- **seats.aero**
  [docs: <https://docs.seats.aero/article/68-seatsaero-pro-api-access-limits-and-usage>,
  <https://docs.seats.aero/article/80-does-seats-aero-support-latam-or-latam-pass-awards>]:
  - Covers GOL Smiles and Azul Fidelidade, not LATAM Pass.
  - The Pro API is for personal, non-commercial use: 1,000 calls a day; cached search,
    bulk availability, trips and routes, with no live search; header
    `Partner-Authorization: pro_…`.
  - It is not offered in every country or to every Pro user: check Settings → API after
    subscribing.
  - Pro alerts watch routes without code.
  - Price: $12.99 a month or $109.99 a year from 2026-11-09
    [secondary: <https://milesopedia.com/en/news/travel/seats-aero-pro-price-increase/>].
- **LATAM Pass:** no consumer API was found in this research. B2B options found were
  [APIViagens](https://www.apiviagens.com.br/),
  [GeckoAPI](https://geckoapi.com.br/blog/api-milhas-aereas-smiles-azul-pontos/) and
  [BuscaMilhas](https://buscamilhas.com/api-passagens-aereas/), with pricing on request.
  [EconoMilha](https://www.economilha.com/en) advertises web searches across programs
  [secondary; none of these services was exercised].
- **Direct scraping** of Smiles, LATAM or Azul was not tested. Review each site's current
  terms and access restrictions before considering it as a data source.
- **Transfer bonuses** (Livelo or Esfera into Smiles, Azul or LATAM Pass) are posted on
  the Melhores Destinos Telegram.

---

## 5. A fare monitor for this repo (design, not built)

The goal: a push notification when anything from São Paulo is anomalously cheap, such as
Cape Town at half its usual price.

```
systemd timer, every 4 h (RandomizedDelaySec=45m, Persistent=true)
└─ flightwatch (Python; DynamicUser; StateDirectory=flightwatch)
   ├─ Google deals page   1 GET per run: 30 deals with usual price and % below
   ├─ Google Explore      headless Chromium; ~12 calls per run (6 months × 1- and 2-week trips),
   │                      ~70 destinations each
   ├─ Kiwi MCP            flyTo in {anywhere, Europe, Africa, Asia, North America, Oceania,
   │                      South America}; ~14 calls per run
   ├─ SQLite (WAL)        observations, baselines, alerts
   ├─ detect              rules A–D below
   ├─ confirm             re-query each candidate on exact dates (fli or Kiwi);
   │                      require within +10% AND the alert rule still holds
   └─ notify              Telegram bot or ntfy
```

**Roles.**
- **The deals page is the highest-signal source:** Google already computes the usual
  price.
- **Explore gives breadth**, and feeds our own baseline beyond Google's top 30.
- **Kiwi is an independent second inventory** (including virtual interlining and Azul
  from VCP) and the cross-check.
- **fli confirms and drills down;** don't poll with it.
- **Optionally,** ingest the Melhores Destinos RSS and Telegram preview for human-curated
  and error fares.

**Storage.**
- `observations(ts, source, origin, dest, dest_kgmid, dest_name, country, depart, return,
  nights, price_brl, usual_brl, pct_below, airline, stops, duration_min, link)`
- `baseline(dest, month, nights_bucket, n, median, mad, p10, updated_at)`, rebuilt daily
- `alerts(dest, depart, return, price_brl, reason, sent_at)`, for dedup

At 1–2k rows per run and six runs per day, this is about 2.2–4.4 million rows per year
before deduplication. Measure row and index sizes and choose retention before deployment.

**Alert** when a candidate passes any of:

- **A, Google says so:** `pct_below ≥ 40` (Google's own threshold is 20), and the
  destination is international or on a watch list.
- **B, our history:** bucket by destination, travel month and 5–9 or 10–16 nights. Once a
  bucket has at least 20 observations over at least 14 days, fire when MAD > 0,
  robust z = (median − price) / (1.4826 · MAD) ≥ 3 and price ≤ 0.7 · median.
  If MAD is zero, skip this rule and use a documented typical price or target cap.
- **C, cold start:** if Google supplies a usual price, apply rule A's 40% threshold.
  Otherwise, if the paid fallback supplies `typical_price_range=[low, high]` with
  `0 < low ≤ high`, require price ≤ 0.7 · low. Without either baseline, do not claim
  an anomalous discount: use rule D or keep the fare as a discovery candidate.
- **D, target prices:** per-destination caps, e.g. Cape Town round trip ≤ R$3,000.

After confirmation, re-evaluate the qualifying rule against the new price and compare
like-for-like passenger count, cabin, airports, dates and stops. The +10% tolerance never
overrides a target cap or discount threshold. A 2,990 quote repriced to 3,289 must not
trigger a 3,000 target-price alert.

Send one alert per destination and ±3 days, and re-alert only on a further ≥ 5% drop. For
calibration: on 2026-11-09, Cape Town round trips were R$4.2–4.65k across Google, Kiwi
and fli for December–March, against a Google usual price of R$7.8–8.6k. Rule A would have
fired.

**NixOS integration requirements**, not an installable module:

- Package the program before defining `ExecStart`; there is no `flightwatch` package here.
- Use a `Type=oneshot` service, `DynamicUser=true`, `StateDirectory=flightwatch` and
  `pkgs.chromium` on its PATH.
- Provision a Telegram credential through agenix and register its recipients in
  `secrets.nix`. Follow `modules/services/buildbot-secrets.nix`: use the secret's
  `config.age.secrets.<name>.path` as a systemd `LoadCredential` source, never a Nix-store
  string containing the token. The program reads it from `$CREDENTIALS_DIRECTORY`.
- Attach a timer with `OnCalendar=*-*-* 00/4:00:00`, `RandomizedDelaySec=45m` and
  `Persistent=true`. Select the host and notification destination before deployment.

**Operations.**
- **Host:** run it on a machine that stays in São Paulo and off VPN, since the deals and
  Explore pages pick the origin from the IP.
- **Volume:** about 160 requests a day across Google and Kiwi.
- **Notifications:**
  - Telegram: `POST https://api.telegram.org/bot<TOKEN>/sendMessage` with `chat_id` and
    `text`.
  - ntfy: publish over HTTPS to an authenticated private topic, with publisher and
    subscriber access controls; an obscure public topic name is not authentication.
    Self-hosting is available through `services.ntfy-sh`.
- **Cost:** nothing. Optional extras: seats.aero Pro for miles, and SerpApi's free tier
  for calibration.
- **Failure mode:** Google's undocumented page and RPC formats can change; fli documents
  a change in August 2026. Wrap each source in an adapter that checks its schema, and
  alert ourselves when a source returns no rows twice in a row. Kiwi stays as a second
  source.

**Paid fallback, documented but not exercised:** use SerpApi's Explore and per-route
insights where available; this is not a verified equivalent of the Deals page.
- `google_travel_explore` with `departure_id=GRU,CGH,VCP`, `currency=BRL`, `gl=br`,
  `month`, `travel_duration` and `arrival_area_id` per region.
- [`google_flights` price insights](https://serpapi.com/google-flights-price-insights)
  for each candidate. Their typical price range is a lower/upper pair, not the Deals
  page's usual-price scalar. Disable rule A unless the actual Deals inputs remain
  available; use rule C's explicit lower-bound comparison instead.
- About 15 calls per run: 4 runs a day is about 1,800 a month, the $75 Developer plan; 1–2
  runs a day fits the $25 Starter plan.

---

## 6. Worked example: São Paulo ⇄ Brasília, January 2027

**Illustrative search criteria:** CGH or GRU to BSB and back, about a week, any time in
January 2027. Prefer CGH, a Friday-night or weekend departure and a Sunday return.
These are alternative market quotes, not a confirmed personal itinerary.

**Method:**
1. Run the §3.3 calendar for every January departure, with stays of 5–10 nights, from
   both airports.
2. Run live searches (§3.4) on the cheapest shapes, clicking through for return times.
3. Run one-way searches to price split tickets.

The live options below were nonstop economy quotes for one adult in BRL, with required
taxes and fees included. Airline checkout, baggage allowance and seat fees were not
verified. Calendar queries used no nonstop filter. Times are local.

**Cheapest round trip by departure date** (calendar, exactly 7 nights; other trip
lengths can cost more or less):

| Departure | CGH, Nov 9 | CGH, Nov 11 | GRU, Nov 9 | GRU, Nov 11 |
|---|---|---|---|---|
| Fri Jan 1 | 1,505 | 1,395 | 1,511 | 1,542 |
| Sat Jan 2 | 1,501 | 1,446 | 1,408 | 1,480 |
| Sun Jan 3 | 2,283 | 1,980 | 2,595 | 1,881 |
| Mon Jan 4 | 2,303 | 1,772 | 1,602 | 1,594 |
| Tue Jan 5 | 1,066 | 1,187 | 1,018 | 1,018 |
| Wed Jan 6 | 938 | 936 | 1,018 | 1,018 |
| Thu Jan 7 | 847 | 808 | 1,018 | 1,018 |
| Fri Jan 8 | 737 | 808 | 711 | 1,018 |
| Sat Jan 9 | 737 | 808 | 711 | 1,018 |
| Sun Jan 10 – Wed Jan 27 | 737 | 699 | 711 | 711 |

**Options found on 2026-11-11** (live, 14:44–15:02):

| Trip | Nights | Tickets | Out, CGH → BSB | Back, BSB → CGH | Total |
|---|---|---|---|---|---|
| Fri Jan 15 → Sun Jan 24 | 9 | Gol round trip | 17:10, 20:25 | 06:00, 09:50, 13:30, 15:05, 17:15, 18:50, 20:30 | 699 |
| Sat Jan 16 → Sun Jan 24 | 8 | Gol round trip | 18:20 | same as above | 699 |
| Sun Jan 10 → Sun Jan 17 | 7 | Gol round trip | 06:10 | 06:00, 09:00, 09:50, 13:30, 15:05, 17:15, 18:50, 20:30 | 699 |
| Sun Jan 10 → Sun Jan 17 | 7 | LATAM round trip | 10:40 | 09:35, 13:05, 18:40, 19:35 | 737 |
| Fri Jan 8 → Sun Jan 17 | 9 | two one-ways | Azul 18:35 (456) | LATAM 18:40, 19:35 or Gol 17:15, 18:50, 20:30 and earlier (355) | 811 |
| Sat Jan 9 → Sun Jan 17 | 8 | two one-ways | Azul 17:30, 21:45 (456) | as above (355) | 811 |
| Fri Jan 8 or Sat Jan 9 → Sun Jan 17 | 9 or 8 | Azul round trip | Fri 18:35 or Sat 17:30 | 05:50, 16:55 | 837 |
| Fri Jan 8 → Sun Jan 17 | 9 | LATAM round trip | 19:10, 21:10 | 09:35, 13:05, 18:40, 19:35 | 938 |

What it showed:

- **Longer stays sometimes cost the same, not always.** In the November 11 calendar,
  CGH departures on January 10–24 stayed at 699 for every duration from 5–10 nights.
  But January 8 cost 808 for 7 nights and 837 for 9; January 1 cost 1,912 for 5 nights
  and 1,030 for 9. Compare each date pair rather than assuming extra nights are free.
- **Prices moved a lot in two days.** LATAM's January 8 Friday-night round trip went
  from 737 on 2026-11-09 to 938. GRU's January 8 departures at 06:00, 14:55 and 23:40
  had advertised round-trip minimums of 711, then 1,018 two days later. CGH seven-night
  calendar prices for January 10–27 fell from 737 to 699; later dates had exceptions.
  Re-run §3.3 to find candidates, then check live results before buying.
- **Split tickets can win.** On 2026-11-11, an Azul one-way out plus a LATAM or Gol one-way
  back (811) beat both Azul's round trip (837) and LATAM's (938) for the first weekend. On
  2026-11-09, LATAM's January 9–16 round trip was 737, against 477 + 449 for its
  one-ways. Separate tickets are separate bookings: changes or cancellation of one
  do not automatically change or refund the other.
- **Calendar and live can disagree on price.** The calendar is a discovery aid;
  select both flights to identify the actual combination and quote, then confirm
  the payable amount and fare conditions with the seller.
- **Google's booking advice changed too.** On 2026-11-09 it said the cheapest time to book
  was "usually now, until Nov 11". On 2026-11-11 it said prices were typical and the
  cheapest time was "usually earlier, about 2–5 months before takeoff".
