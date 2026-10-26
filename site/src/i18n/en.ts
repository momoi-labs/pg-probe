// The English text of the site, the source of the other languages. A value
// with markup is rendered with set:html, so it may hold <b> and <code>.
const en = {
  meta: {
    title: 'pg-probe: PostgreSQL diagnostics in one file',
    description:
      'Your client runs one command on their PostgreSQL server and sends you one file. Review it in a local console and print a report in English, Portuguese, or Spanish.',
  },
  language: {
    label: 'Language',
    browser: 'Browser language',
  },
  nav: {
    how: 'How it works',
    report: 'Report',
    top: 'Live metrics',
    agent: 'AI agent',
  },
  hero: {
    eyebrow: 'For PostgreSQL engineers',
    title: 'Your client runs one command. You get the full picture.',
    lead: 'pg-probe records PostgreSQL and Linux host metrics into one file. Open it in a local console, or print a report your client can read.',
    install: 'Install pg-probe',
    howEyebrow: 'How it works',
    howTitle: 'Capture there. Review here.',
  },
  flow: {
    label: 'How pg-probe is used',
    capture: 'Capture',
    captureWhere: "On the client's server",
    handoff: 'The file changes hands',
    handoffWhere: 'outside pg-probe',
    review: 'Review',
    reviewWhere: 'On your laptop',
    captureNote: 'The output is one file in the current directory. Nothing leaves the server.',
    terminal: 'Terminal, on your laptop',
    consoleTitle: 'The pg-probe review console',
    reviewNote: 'Open the address in your browser. The console reads the same file, on your machine.',
  },
  handoff: {
    from: 'From',
    to: 'To',
    you: 'you',
    subject: 'Subject',
    subjectText: 'Capture from db-prod-01',
    body: "Here is the capture from this morning's slowdown.",
    note: 'Not a pg-probe step. Your client sends the file the way they send any file: email, ticket, or shared folder. pg-probe uploads nothing.',
  },
  report: {
    eyebrow: 'The printed report',
    title: 'A report your client can read.',
    lead: 'Print the whole session to PDF from the console: 39 charts of the host and the database, from CPU and disks to locks and WAL. Each chart gets its own page, with what it measures and how to read it.',
    ticks: [
      '<b>Cover and contents</b>, linked to every chart',
      '<b>"What to look for"</b> on each page, written for a reader who does not know the metric',
      '<b>Sources that were not available</b> listed after the cover, never drawn as zero',
    ],
    languages: 'In English, Portuguese, or Spanish, whichever your client reads.',
    sample: 'Open the sample report',
    sampleNote: 'PDF, 47 pages, in a new tab.',
    coverAlt: 'Report cover',
    pageAlt: 'A report page: lock requests waiting, with a What to look for box above the chart',
  },
  console: {
    eyebrow: 'The console',
    title: 'Built for reading a session.',
    lead: 'Click a chart to pin one moment, or drag to select a range. Every chart and every legend follows.',
    fine: 'From the demo capture: pinned at 15:04:34 UTC, transactions fell to 2.6 per second while 32 lock requests waited, and the CPU was 3.4% busy.',
    features: [
      { title: 'Percentiles, not averages', text: 'Every legend shows p50, p75, p95, p99, and the current value.' },
      {
        title: 'Rates from samples',
        text: '<code>--duration</code> and <code>--interval</code> record how counters change. A counter reset is a gap, never a value.',
      },
      {
        title: 'Your own SQL',
        text: 'The artifact is plain SQLite. Open it with <code>sqlite3</code> when the console is not enough.',
      },
    ],
  },
  agent: {
    eyebrow: 'Ask an AI agent',
    title: 'Let your AI agent read the file.',
    lead: '<code>pg-probe mcp</code> gives Claude Code, Codex, or any agent that speaks MCP read-only access to the snapshots in a directory. Ask in plain language, for example "Did any collector fail? Why?". The agent writes the SQL and cites the table behind each answer.',
    copy: 'Copy the command',
    commandNote:
      'For Codex, run <code>codex mcp add</code> with the same arguments. Other agents take <code>pg-probe mcp -D &lt;directory&gt;</code> in their MCP settings.',
    features: [
      {
        title: 'Knows the schema',
        text: 'Every table and column comes with its unit and its source, and every collector error code with what it means.',
      },
      {
        title: 'Read-only SQL',
        text: 'One statement at a time, up to 1,000 rows, with <code>percentile()</code> for p95 and p99. Larger results go to a new CSV file.',
      },
      {
        title: 'Runs on your laptop',
        text: 'The server runs locally and never connects to PostgreSQL. What the agent reads goes to the model it uses.',
      },
    ],
  },
  security: {
    eyebrow: "For your client's security team",
    title: 'No agent. No account. No upload.',
    lead: 'Your client does not install a monitoring stack, and you do not need access to their server.',
    features: [
      {
        title: 'Read-only session',
        text: 'One connection with <code>default_transaction_read_only=on</code> and a statement timeout.',
      },
      {
        title: 'Strict allowlist',
        text: 'No credentials, environment variables, command lines, or network identifiers in the file.',
      },
      {
        title: 'Inspectable',
        text: 'Your client can open the file before sending it. Nothing leaves the server on its own.',
      },
    ],
  },
  top: {
    eyebrow: 'Also in the binary',
    title: 'What is the database doing right now?',
    copy: 'Copy the command',
    lead: 'See the queries running now, longest first, and what each one waits on. It works with PostgreSQL 9.3 and newer.',
    views: 'pg-probe top views',
    running: 'Running',
    runningNote: 'Every client backend that is not idle, longest first. On the database host, also its CPU, I/O, and I/O wait from <code>/proc</code>.',
    waiting: 'Waiting',
    waitingNote: 'Backends waiting on a lock, and the lock each one waits on.',
    blocking: 'Blocking',
    blockingNote: 'Who holds the lock, with the queue behind each holder as a tree.',
    vacuum: 'Vacuum',
    vacuumNote: 'Every autovacuum and manual vacuum. From PostgreSQL 9.6, also its phase and how much of the table it has scanned.',
    query: 'Query',
    queryNote: 'One backend: the full query and every age. On the database host, also its CPU, memory, and I/O.',
    plan: 'Plan',
    planNote: 'The estimated plan. <code>EXPLAIN</code> without <code>ANALYZE</code>, so the query does not run again.',
    function: 'Function',
    functionNote: 'The plpgsql function it calls: signature, call statistics, and source.',
    cost: 'What it costs',
    connection: 'read-only connection',
    refresh: 'per refresh',
    core: 'of one core',
    costFine:
      'Measured on a test server with 206 connections under pgbench. One SELECT over pg_stat_activity per refresh. The lock views, the query page, and the popups add one query each, only while open.',
  },
  install: {
    eyebrow: 'Install',
    title: 'Runs on the servers your clients have.',
    postgres: '9.3 and newer',
    postgresText: 'PostgreSQL versions',
    el: 'EL7 and newer',
    elText: 'RHEL, CentOS, Rocky. rpm, tested on Rocky Linux 9',
    ubuntu: 'Ubuntu 18.04 and newer',
    ubuntuText: 'deb, tested on Ubuntu 24.04',
    archText: 'static binary, no dependencies',
    copy: 'Copy the install command',
    download: 'Download for Linux',
    downloadText: 'deb, rpm, or a static binary, with no dependencies.',
    fine: 'Open source under the PostgreSQL License. The capture, the console, and the report ship in the same binary.',
  },
  footer: {
    license: 'PostgreSQL License',
    releases: 'Releases',
  },
};

export default en;
export type Dictionary = typeof en;
