import type { Translation } from '../i18n.ts'

const translation: Translation = {
  messages: {
    ', every sample': ', em todas as amostras',
    '1 minute': '1 minuto',
    '1 sample': '1 amostra',
    '1 source failed': '1 fonte falhou',
    '1 source is not available on this host or server version':
      '1 fonte não está disponível neste host ou nesta versão do servidor',
    '15 minutes': '15 minutos',
    '5 minutes': '5 minutos',
    Abandoned: 'Abandonadas',
    'Active backends, by wait event type': 'Backends ativos, por tipo de evento de espera',
    'All databases': 'Todos os bancos de dados',
    'All {total} sources': 'Todas as {total} fontes',
    Archived: 'Arquivados',
    'Artifacts do not record this today, so there is nothing to show for this snapshot.':
      'Os artefatos ainda não registram esse dado, então não há nada a mostrar neste snapshot.',
    'Average read latency, five busiest disks (ms)':
      'Latência média de leitura, cinco discos mais ocupados (ms)',
    'Average requests in flight, five busiest disks':
      'Média de requisições em andamento, cinco discos mais ocupados',
    'Average write latency, five busiest disks (ms)':
      'Latência média de gravação, cinco discos mais ocupados (ms)',
    'Backend writes': 'Gravações dos backends',
    'Background writer': 'Background writer',
    'Block I/O time': 'Tempo de I/O de blocos',
    'Blocked on I/O': 'Bloqueados em I/O',
    'Buffer pin': 'Buffer pin',
    Buffers: 'Buffers',
    CPU: 'CPU',
    'CPU busy': 'CPU ocupada',
    'CPU utilization': 'Utilização de CPU',
    'CPU utilization by state (%)': 'Utilização de CPU por estado (%)',
    CPUs: 'CPUs',
    'Cache hit ratio': 'Taxa de acerto do cache',
    'Cache hit ratio per interval (%)': 'Taxa de acerto do cache por intervalo (%)',
    Capture: 'Captura',
    'Cgroup I/O': 'I/O do cgroup',
    'Cgroup I/O, all devices (MiB/s)': 'I/O do cgroup, todos os dispositivos (MiB/s)',
    'Cgroup events': 'Eventos do cgroup',
    'Cgroup limit events per interval': 'Eventos de limite do cgroup por intervalo',
    'Cgroup memory': 'Memória do cgroup',
    'Charts cover': 'Os gráficos cobrem',
    'Checkpoint writes': 'Gravações do checkpoint',
    'Checksum failures': 'Falhas de checksum',
    'Client connections by state': 'Conexões de clientes por estado',
    'Cluster size': 'Tamanho do cluster',
    'Collected in full.': 'Coletadas por completo.',
    'Collected in part': 'Coletadas em parte',
    'Collected, {count} of them in part. See page 2.':
      'Coletadas, {count} delas em parte. Veja a página 2.',
    'Collected. See page 2.': 'Coletadas. Veja a página 2.',
    Commits: 'Commits',
    Complete: 'Completa',
    'Connected as': 'Conectado como',
    Connections: 'Conexões',
    'Connections by state': 'Conexões por estado',
    Contents: 'Sumário',
    Current: 'Atual',
    'Data coverage': 'Cobertura dos dados',
    'Database size': 'Tamanho dos bancos de dados',
    'Dead (%)': 'Mortas (%)',
    'Dead tuples': 'Tuplas mortas',
    Deadlock: 'Deadlock',
    Deadlocks: 'Deadlocks',
    'Deadlocks and conflicts': 'Deadlocks e conflitos',
    'Deadlocks and recovery conflicts per interval':
      'Deadlocks e conflitos de recuperação por intervalo',
    'Deadlocks, recovery conflicts and checksum failures per interval':
      'Deadlocks, conflitos de recuperação e falhas de checksum por intervalo',
    'Direct, by the allocating process': 'Direta, pelo processo que aloca',
    'Disk busy': 'Disco ocupado',
    'Disk busy time, five busiest disks (%)': 'Tempo ocupado dos discos, cinco discos mais ocupados (%)',
    'Disk queue depth': 'Profundidade da fila de disco',
    'Disk read latency': 'Latência de leitura de disco',
    'Disk throughput': 'Vazão de disco',
    'Disk throughput, all disks (MiB/s)': 'Vazão de disco, todos os discos (MiB/s)',
    'Disk utilization': 'Utilização de disco',
    'Disk write latency': 'Latência de gravação de disco',
    Duration: 'Duração',
    'Each value is the last sample, or the last interval for rates. Peaks cover the whole session.':
      'Cada valor é a última amostra, ou o último intervalo no caso de taxas. Os picos cobrem toda a coleta.',
    Endpoint: 'Endpoint',
    Error: 'Erro',
    Extends: 'Extensões',
    Failed: 'Falhas',
    Fatal: 'Fatal',
    'Filesystem usage': 'Uso do sistema de arquivos',
    Free: 'Livre',
    Host: 'Host',
    'I/O': 'I/O',
    'I/O by operation': 'I/O por operação',
    'I/O by operation, all backends (MiB/s)': 'I/O por operação, todos os backends (MiB/s)',
    'I/O wait': 'Espera de I/O',
    'It also needs track_io_timing on the server.': 'Também requer track_io_timing ligado no servidor.',
    Kernel: 'Kernel',
    Killed: 'Terminadas',
    'Largest tables': 'Maiores tabelas',
    'Largest {type} slot': 'Maior slot {type}',
    'Last autovacuum': 'Último autovacuum',
    'Live tuples': 'Tuplas vivas',
    'Load average': 'Carga média',
    'Load average, 1 / 5 / 15 minutes': 'Carga média, 1 / 5 / 15 minutos',
    'Load, 1 minute': 'Carga, 1 minuto',
    'Lock requests waiting, by lock type': 'Pedidos de lock em espera, por tipo de lock',
    'Lock waits': 'Esperas por lock',
    'Locks held': 'Locks mantidos',
    'Locks held, by mode': 'Locks mantidos, por modo',
    Logs: 'Logs',
    'Longest lock wait': 'Maior espera por lock',
    'Longest lock wait (s)': 'Maior espera por lock (s)',
    'Longest time in state': 'Maior tempo no estado',
    'Longest time in state since state_change (min)': 'Maior tempo no estado desde state_change (min)',
    'Longest wait': 'Maior espera',
    'Lowest {value} at {time}': 'Mínimo de {value} às {time}',
    'Major faults': 'Falhas maiores',
    'Major page faults': 'Falhas de página maiores',
    'Major page faults per second': 'Falhas de página maiores por segundo',
    'Max lag': 'Lag máximo',
    Memory: 'Memória',
    'Memory and swap charged to the cgroup (GiB)': 'Memória e swap atribuídos ao cgroup (GiB)',
    'Memory by use, up to MemTotal (GiB)': 'Memória por uso, até MemTotal (GiB)',
    'Memory used': 'Memória usada',
    Metrics: 'Métricas',
    'Most expensive statements': 'Comandos mais caros',
    'Needs a new postgres.stats.statements collector, after the decision on whether query text may enter the artifact.':
      'Requer um novo coletor postgres.stats.statements, depois da decisão sobre se o texto das consultas pode entrar no artefato.',
    'Network drops and errors': 'Descartes e erros de rede',
    'Network throughput': 'Vazão de rede',
    'Network throughput, all interfaces (MiB/s)': 'Vazão de rede, todas as interfaces (MiB/s)',
    'Nice, IRQ, softirq': 'Nice, IRQ, softirq',
    'No collected samples in this window.': 'Nenhuma amostra coletada nesta janela.',
    'No dead tuples in the recorded tables.': 'Nenhuma tupla morta nas tabelas registradas.',
    'No measurements.': 'Sem medições.',
    'No standby connected': 'Nenhum standby conectado',
    'No standby was connected in the last sample.': 'Nenhum standby estava conectado na última amostra.',
    None: 'Nenhum',
    'Not available on this host or version': 'Não disponíveis neste host ou nesta versão',
    'Not collected': 'Não coletado',
    'Not recorded in this artifact.': 'Não registrado neste artefato.',
    'Not recorded in this artifact. Captures before schema version 3 have no mount sizes.':
      'Não registrado neste artefato. Capturas anteriores à versão 3 do schema não têm o tamanho dos pontos de montagem.',
    'Not recorded in this artifact. Captures before schema version 3 have no network counters.':
      'Não registrado neste artefato. Capturas anteriores à versão 3 do schema não têm contadores de rede.',
    'Not recorded in this artifact. Captures before schema version 3 have no relations.':
      'Não registrado neste artefato. Capturas anteriores à versão 3 do schema não têm relações.',
    'Not recorded in this artifact: proc.pressure is {reason}. Older kernels and some containers have no PSI.':
      'Não registrado neste artefato: o status de proc.pressure é {reason}. Kernels antigos e alguns contêineres não têm PSI.',
    'Not recorded in this artifact: {reason}.': 'Não registrado neste artefato: {reason}.',
    Other: 'Outros',
    'Other states': 'Outros estados',
    Overview: 'Visão geral',
    'Packets dropped and errors per second, all interfaces':
      'Pacotes descartados e erros por segundo, todas as interfaces',
    'Page cache': 'Cache de páginas',
    'Page reclaim': 'Recuperação de páginas',
    'Pages scanned for reclaim per second': 'Páginas varridas para recuperação por segundo',
    Partial: 'Parcial',
    'Peak {value} at {time}': 'Pico de {value} às {time}',
    PostgreSQL: 'PostgreSQL',
    'PostgreSQL capture report': 'Relatório de captura do PostgreSQL',
    'PostgreSQL panels': 'Painéis do PostgreSQL',
    'PostgreSQL {version} does not have these statistics':
      'O PostgreSQL {version} não tem essas estatísticas',
    'Pressure stall': 'Pressão (PSI)',
    'Pressure stall, some tasks, 10 s average (%)': 'Pressão (PSI), algumas tarefas, média de 10 s (%)',
    Processes: 'Processos',
    'Processes running and blocked': 'Processos em execução e bloqueados',
    Queries: 'Consultas',
    'Queries canceled by recovery conflicts per interval':
      'Consultas canceladas por conflitos de recuperação, por intervalo',
    'Query latency': 'Latência das consultas',
    'Query panels': 'Painéis de consultas',
    'Query source': 'Fonte dos dados de consultas',
    'Query volume': 'Volume de consultas',
    Read: 'Leitura',
    Reads: 'Leituras',
    Reason: 'Motivo',
    Received: 'Recebidos',
    'Received, dropped': 'Recebidos, descartados',
    'Received, errors': 'Recebidos, erros',
    'Recovery conflicts': 'Conflitos de recuperação',
    'Replica lag': 'Lag da réplica',
    'Replication lag': 'Lag de replicação',
    'Replication slots': 'Slots de replicação',
    Rollbacks: 'Rollbacks',
    Rows: 'Linhas',
    'Rows per second': 'Linhas por segundo',
    Running: 'Em execução',
    'SLRU blocks read from disk per second': 'Blocos SLRU lidos do disco por segundo',
    'SLRU reads': 'Leituras de SLRU',
    Samples: 'Amostras',
    Sent: 'Enviados',
    'Sent, dropped': 'Enviados, descartados',
    'Sent, errors': 'Enviados, erros',
    Series: 'Série',
    'Sessions ended abnormally': 'Sessões encerradas de forma anormal',
    'Sessions ended abnormally per interval': 'Sessões encerradas de forma anormal por intervalo',
    'Shared memory': 'Memória compartilhada',
    'Size per database': 'Tamanho por banco de dados',
    'Size per table (GiB)': 'Tamanho por tabela (GiB)',
    Snapshot: 'Snapshot',
    'Snapshot sections': 'Seções do snapshot',
    'Some databases do not accept connections': 'Alguns bancos de dados não aceitam conexões',
    'Some databases were not visited: {reason}.': 'Alguns bancos de dados não foram visitados: {reason}.',
    'Some of the data came in. The rest is expected to be missing.':
      'Parte dos dados foi coletada. É esperado que o resto falte.',
    Source: 'Fonte',
    Sources: 'Fontes',
    'Standby conflicts': 'Conflitos no standby',
    Standbys: 'Standbys',
    State: 'Estado',
    Status: 'Status',
    Steal: 'Steal',
    Swap: 'Swap',
    'Swap activity': 'Atividade de swap',
    'Swap in use': 'Swap em uso',
    'Swap used (GiB)': 'Swap em uso (GiB)',
    'Swapped in': 'Lido do swap',
    'Swapped in and out (MiB/s)': 'Leitura e gravação de swap (MiB/s)',
    'Swapped out': 'Gravado no swap',
    'Sync state': 'Estado de sincronização',
    System: 'Sistema',
    'System panels': 'Painéis do sistema',
    Table: 'Tabela',
    Tablespace: 'Tablespace',
    'Temporary files written': 'Arquivos temporários gravados',
    'Temporary files written (MiB/s)': 'Arquivos temporários gravados (MiB/s)',
    'The PostgreSQL cgroup has no I/O statistics':
      'O cgroup do PostgreSQL não tem estatísticas de I/O',
    'The PostgreSQL cgroup has no events file':
      'O cgroup do PostgreSQL não tem o arquivo de eventos',
    'The capture ran to the end.': 'A captura foi até o fim.',
    'The capture stopped early, or a source failed.':
      'A captura parou antes do fim, ou uma fonte falhou.',
    'The host has no /etc/os-release': 'O host não tem /etc/os-release',
    'The kernel has no pressure stall information (PSI)':
      'O kernel não tem pressure stall information (PSI)',
    'The kernel has no transparent huge pages settings':
      'O kernel não tem as configurações de transparent huge pages',
    'The source exists on this host, but pg-probe could not read it. Fix the cause and capture again.':
      'A fonte existe neste host, mas o pg-probe não conseguiu lê-la. Corrija a causa e capture de novo.',
    'The whole session': 'A coleta inteira',
    'These databases do not accept connections: {names}':
      'Estes bancos de dados não aceitam conexões: {names}',
    'These sources have no data in this artifact. Panels that read them show an unknown value.':
      'Estas fontes não têm dados neste artefato. Os painéis que dependem delas mostram um valor desconhecido.',
    'This PostgreSQL version does not have these statistics':
      'Esta versão do PostgreSQL não tem essas estatísticas',
    'This kernel or server version does not have these sources. Nothing to fix.':
      'Este kernel ou esta versão do servidor não tem essas fontes. Não há nada a corrigir.',
    'Time reading and writing blocks (ms per second)': 'Tempo lendo e gravando blocos (ms por segundo)',
    'Time split': 'Divisão do tempo',
    Timestamp: 'Data e hora',
    Tool: 'Ferramenta',
    Transactions: 'Transações',
    'Transactions per second': 'Transações por segundo',
    'Unknown: {reason}.': 'Desconhecido: {reason}.',
    'Unknown, {reason}': 'Desconhecido, {reason}',
    'Not waiting': 'Sem espera',
    Used: 'Em uso',
    User: 'Usuário',
    Version: 'Versão',
    'View exact values': 'Ver valores exatos',
    'WAL and buffer writes (MiB/s)': 'Gravações de WAL e de buffers (MiB/s)',
    'WAL and checkpoint writes': 'Gravações de WAL e checkpoint',
    'WAL archiving': 'Arquivamento de WAL',
    'WAL files archived per interval': 'Arquivos WAL arquivados por intervalo',
    'WAL generated': 'WAL gerado',
    'WAL retained by replication slots (MiB)': 'WAL retido por slots de replicação (MiB)',
    'Wait events': 'Eventos de espera',
    Waiting: 'Em espera',
    'What to look for': 'O que observar',
    Why: 'Motivo',
    'Worst standby': 'Pior standby',
    Write: 'Gravação',
    Writes: 'Gravações',
    Written: 'Gravação',
    'a rate needs two or more samples, and this artifact holds one':
      'uma taxa precisa de duas ou mais amostras, e este artefato tem só uma',
    'all interfaces': 'todas as interfaces',
    blocked: 'bloqueado',
    'by total time in the window': 'por tempo total na janela',
    'calls per second': 'chamadas por segundo',
    'captured {time}': 'capturado em {time}',
    'captured {time} in {duration}': 'capturado em {time}, com duração de {duration}',
    'every sample': 'todas as amostras',
    failed: 'falhou',
    'heap, indexes and TOAST, top 6': 'heap, índices e TOAST, top 6',
    'kswapd, in the background': 'kswapd, em segundo plano',
    'last sample, from pg_stat_replication': 'última amostra, de pg_stat_replication',
    'no collector': 'sem coletor',
    'non-I/O against block I/O': 'tempo fora de I/O contra I/O de blocos',
    'not recorded': 'não registrado',
    'not recorded in this artifact': 'não registrado neste artefato',
    'of {max} max_connections': 'de {max} em max_connections',
    'on {distribution}': 'em {distribution}',
    'percentiles across the window': 'percentis na janela',
    permission_denied: 'permissão negada',
    'pg-probe read {total} sources. {full} collected in full. Charts show the rest as unknown, never as zero.':
      'O pg-probe leu {total} fontes. {full} foram coletadas por completo. Os gráficos mostram o resto como desconhecido, nunca como zero.',
    'pg_stat_statements is installed in {databases}. The artifact does not read it yet.':
      'O pg_stat_statements está instalado em {databases}. O artefato ainda não lê esses dados.',
    'pg_stat_statements is not installed in any database the probe visited.':
      'O pg_stat_statements não está instalado em nenhum banco de dados que o pg-probe visitou.',
    'pg_stat_statements stores totals and means, never a distribution. Percentiles need the server log, which the probe does not read.':
      'O pg_stat_statements guarda totais e médias, nunca uma distribuição. Percentis exigem o log do servidor, que o pg-probe não lê.',
    'point in time': 'momento da captura',
    'point in time, local disks': 'momento da captura, discos locais',
    primary: 'primário',
    'since the last statistics reset': 'desde o último reset de estatísticas',
    'some, 10s window': 'some, janela de 10s',
    standby: 'standby',
    'stated, never guessed': 'informado, nunca suposto',
    unsupported: 'sem suporte',
    'vacuum backlog, top 6': 'vacuum pendente, top 6',
    '{collected} of {total} sources': '{collected} de {total} fontes',
    '{count} CPUs online': '{count} CPUs online',
    '{count} of {total}': '{count} de {total}',
    '{count} samples': '{count} amostras',
    '{count} samples, every {step} s': '{count} amostras, a cada {step} s',
    '{count} sources are not available on this host or server version':
      '{count} fontes não estão disponíveis neste host ou nesta versão do servidor',
    '{count} sources failed': '{count} fontes falharam',
    '{date}, {from} to {to} UTC': '{date}, {from} a {to} UTC',
    '{disk}, the busiest disk': '{disk}, o disco mais ocupado',
    '{from} to {to} UTC': '{from} a {to} UTC',
    '{full} of {total} collected in full. See page 2.':
      '{full} de {total} coletadas por completo. Veja a página 2.',
    '{label}: open the {chart} chart': '{label}: abrir o gráfico {chart}',
    '{label}: summary': '{label}: resumo',
    '{used} of {total}': '{used} de {total}',
    '{used} of {total} {unit}': '{used} de {total} {unit}',
    '{user}, superuser': '{user}, superusuário',
    '{what} are not collected yet': '{what} ainda não são coletados',
  },
  explain: {
    'cpu-utilization': {
      shows:
        'Parcela do tempo de CPU em cada estado, na média de todas as CPUs. Usuário é o tempo em programas como os backends do PostgreSQL, e Sistema é o tempo no kernel. Espera de I/O é tempo ocioso enquanto havia uma requisição de disco pendente. Steal é o tempo que o hypervisor deu a outras máquinas virtuais.',
      look: 'Some Usuário e Sistema para obter o tempo ocupado. Quando o tempo ocupado fica acima de 80% por minutos, verifique a Carga média. Uma carga acima do número de CPUs no mesmo período pode indicar que consultas esperaram por um núcleo, mas a carga do Linux também conta tarefas que esperam pelo disco. Um p95 perto de 100% com um p50 baixo na tabela indica picos curtos, não uma falta constante. Espera de I/O acima de 10% indica que as CPUs ficaram ociosas enquanto havia requisições de disco pendentes, então verifique os gráficos de latência. Steal acima de alguns por cento indica que a instância divide suas CPUs com vizinhos ocupados.',
    },
    'load-average': {
      shows:
        'Número médio de tarefas em execução ou esperando para executar, nos últimos 1, 5 e 15 minutos. No Linux, a contagem também inclui tarefas que esperam pelo disco.',
      look: 'Compare a carga com o número de CPUs no indicador Carga, 1 minuto, da Visão geral. Carga acima do número de CPUs indica que tarefas esperaram por um núcleo ou por um disco. Uma linha de 1 minuto muito acima da linha de 15 minutos é um pico que acabou de começar. Carga alta com pouco tempo ocupado de CPU indica que as tarefas esperam pelo disco, não pela CPU.',
    },
    memory: {
      shows:
        'Como o host usou a RAM. Em uso é a memória ocupada por programas. Memória compartilhada inclui o shared_buffers do PostgreSQL. Cache de páginas são dados de arquivos que o kernel mantém para leituras rápidas, e Buffers são metadados do sistema de arquivos. Livre é a memória que ninguém usa.',
      look: 'Pouca memória Livre, por si só, é normal, porque o Linux ocupa a memória sobrando com cache de páginas. O sinal de alerta é um cache de páginas que encolhe enquanto Em uso cresce. O PostgreSQL então encontra menos dados na memória e lê mais do disco. Leia este gráfico junto com Taxa de acerto do cache e Swap em uso.',
    },
    'swap-in-use': {
      shows: 'Espaço de swap em uso no host.',
      look: 'Uma linha plana costuma ser dados antigos que o kernel moveu há muito tempo, e não causa dano. Uma linha que sobe durante a coleta indica que faltou memória ao host enquanto a captura rodava. Leitura e gravação de swap mostra se o host lê esses dados de volta, e é isso que deixa as consultas lentas.',
    },
    'disk-throughput': {
      shows: 'Bytes lidos e gravados por segundo em todos os discos.',
      look: 'Uma linha que fica parada no mesmo valor máximo por minutos costuma ser um limite do dispositivo ou do volume na nuvem. Um pico de leitura junto com uma queda na Taxa de acerto do cache indica que as consultas leram dados que não estavam na memória. Picos de gravação a cada poucos minutos costumam ser checkpoints. Compare com o Tempo ocupado dos discos e com os gráficos de latência.',
    },
    'disk-utilization': {
      shows:
        'Parcela do tempo em que cada um dos cinco discos mais ocupados teve pelo menos uma requisição em andamento.',
      look: 'Em um único disco rotativo, valores perto de 100% indicam que o disco está saturado. SSDs, arrays RAID e volumes na nuvem atendem muitas requisições ao mesmo tempo, então podem mostrar 100% e ainda ter folga. Nesses casos, confie mais na latência de leitura e de gravação do que neste gráfico.',
    },
    'pressure-stall': {
      shows:
        'Parcela do tempo em que pelo menos uma tarefa ficou parada esperando por CPU, memória ou I/O, em janelas de 10 segundos. O kernel informa esse valor em /proc/pressure.',
      look: 'Valores perto de zero são saudáveis. Um valor que fica acima de 10% mostra qual recurso segurou o trabalho. Pressão de memória acima de zero indica que tarefas esperaram enquanto o kernel liberava memória. Pressão de I/O costuma coincidir com latência de disco alta.',
    },
    'filesystem-usage': {
      shows: 'Espaço usado em cada sistema de arquivos local no momento da captura.',
      look: 'Acima de 85% no sistema de arquivos de dados ou de WAL, sobra pouco espaço para crescimento, arquivos temporários e rajadas de WAL. Se o sistema de arquivos do WAL encher, o PostgreSQL é desligado.',
    },
    'network-throughput': {
      shows: 'Bytes recebidos e enviados por segundo, somando todas as interfaces.',
      look: 'Um teto plano perto da velocidade do link indica que a rede é o limite. Um link de 1 Gbit/s chega a cerca de 119 MiB/s. Picos repentinos costumam coincidir com backups, um standby recuperando o atraso ou consultas que retornam resultados grandes. O tráfego enviado por um primário cresce a cada standby.',
    },
    processes: {
      shows:
        'Processos em execução em uma CPU, e processos bloqueados, o que no Linux quase sempre significa esperando pelo disco.',
      look: 'Em execução acima do número de CPUs indica que tarefas esperaram por um núcleo. Bloqueados em I/O acima de zero por várias amostras seguidas indica que tarefas esperaram pelo armazenamento. Compare com Espera de I/O em Utilização de CPU e com os gráficos de latência de disco.',
    },
    'swap-activity': {
      shows: 'Dados que o kernel moveu da RAM para o swap, e de volta do swap para a RAM, por segundo.',
      look: 'Lido do swap é a linha que prejudica, porque um processo fica parado até cada página voltar do disco. Qualquer leitura constante do swap durante a coleta deixa o PostgreSQL lento. Gravado no swap, por si só, indica que o kernel está liberando espaço, muitas vezes para o cache de páginas.',
    },
    'major-page-faults': {
      shows: 'Falhas de página por segundo que precisaram de uma leitura de disco, porque a página não estava na memória.',
      look: 'Uma taxa baixa e estável é normal quando programas iniciam ou leem arquivos mapeados em memória. Picos que coincidem com leitura do swap ou com um cache de páginas que encolhe indicam que faltou memória ao host.',
    },
    'page-reclaim': {
      shows:
        'Páginas que o kernel varreu por segundo para liberar memória. O kswapd varre em segundo plano. A recuperação direta acontece quando um processo precisa de memória e tem que liberar parte dela por conta própria antes de continuar.',
      look: 'Alguma atividade do kswapd é normal em um host ocupado. Recuperação direta acima de zero indica que processos, incluindo backends do PostgreSQL, ficaram parados enquanto pediam memória. Isso causa picos de latência mesmo quando o swap fica estável.',
    },
    'network-drops-and-errors': {
      shows: 'Pacotes descartados e erros de interface por segundo, recebidos e enviados, em todas as interfaces.',
      look: 'Um host saudável mostra zero. Descartes constantes na recepção costumam indicar que os buffers da interface transbordam sob carga. Erros apontam para um problema de cabo, driver ou placa de rede. Cada descarte faz o TCP reenviar os dados, o que deixa clientes e standbys mais lentos.',
    },
    'disk-read-latency': {
      shows: 'Tempo médio de uma requisição de leitura em cada um dos cinco discos mais ocupados, em milissegundos.',
      look: 'SSDs locais costumam responder em menos de 1 ms, volumes na nuvem em 1 a 5 ms e discos rotativos em 5 a 15 ms. Valores na casa das dezenas de ms que persistem indicam que o disco estava sobrecarregado ou limitado. O p95 na tabela mostra o quanto os piores intervalos foram lentos.',
    },
    'disk-write-latency': {
      shows: 'Tempo médio de uma requisição de gravação em cada um dos cinco discos mais ocupados, em milissegundos.',
      look: 'Com synchronous_commit ligado, cada commit espera até seu WAL chegar ao disco. Por isso, latência de gravação alta no disco do WAL deixa toda transação mais lenta. Em SSD ou armazenamento na nuvem, gravações acima de 10 ms deixam os commits lentos a ponto de os usuários perceberem. Picos a cada poucos minutos costumam coincidir com checkpoints.',
    },
    'disk-queue-depth': {
      shows: 'Número médio de requisições em andamento em cada um dos cinco discos mais ocupados.',
      look: 'Uma fila que cresce junto com a latência indica que o disco não deu conta. Uma fila longa com latência baixa não é problema, porque SSDs e volumes na nuvem funcionam melhor com muitas requisições ao mesmo tempo.',
    },
    'cgroup-memory': {
      shows:
        'Memória e swap atribuídos ao cgroup do pg-probe. Quando o pg-probe roda no contêiner ou na unidade systemd do PostgreSQL, esta é a memória desse contêiner.',
      look: 'Perto do limite, o kernel recupera memória dentro do cgroup. No limite, ele mata um processo. O cache de páginas também conta para o limite, então um cgroup pode parecer cheio quando a maior parte é cache.',
    },
    'cgroup-i-o': {
      shows: 'Bytes lidos e gravados por segundo pelo cgroup do pg-probe, em todos os dispositivos.',
      look: 'Um teto plano pode ser um limite de I/O definido no contêiner. Se a Vazão de disco do host inteiro tem folga enquanto esta linha está plana, o limite está no contêiner, não no disco.',
    },
    'cgroup-events': {
      shows:
        'Variação por intervalo de até cinco contadores do cgroup do pg-probe, com nome no formato controlador.contador, de cpu.stat, memory.events e pids.events. Contadores de CPU terminados em usec são microssegundos de tempo de CPU. Os outros contadores contam eventos.',
      look: 'cpu.nr_throttled acima de zero indica que o limite de CPU do cgroup pausou suas tarefas. memory.high conta as vezes em que o kernel limitou o cgroup e recuperou memória no seu limite flexível, o que nunca mata um processo. memory.max conta as vezes em que o cgroup atingiu seu limite rígido. O kernel só mata um processo quando a recuperação falha nesse ponto, e memory.oom_kill conta essas mortes. A morte de qualquer processo do PostgreSQL reinicia o servidor. pids.max indica que o cgroup não conseguiu iniciar um processo, então novas conexões falham.',
    },
    'connections-by-state': {
      shows:
        'Conexões de clientes em cada amostra, por estado. Conexões active executam uma consulta. Conexões idle esperam pelo cliente. Conexões idle in transaction têm uma transação aberta, mas não executam nada.',
      look: 'Conexões active muito acima do número de CPUs indicam que as consultas disputam núcleos. Conexões idle in transaction que se prolongam seguram locks e impedem o vacuum de remover tuplas mortas. Compare o total com max_connections no indicador Conexões. Muitas conexões idle sugerem um pooler de conexões.',
    },
    'cache-hit-ratio': {
      shows:
        'Parcela das leituras de blocos que o PostgreSQL encontrou no shared_buffers, por intervalo, em todos os bancos de dados. O cache de páginas do Linux ainda pode atender uma falha no cache, então uma falha nem sempre é uma leitura de disco.',
      look: 'Cargas OLTP costumam ficar acima de 99%. Quedas coincidem com varreduras de tabelas grandes, relatórios ou cache frio depois de um restart. Uma taxa que fica baixa enquanto as leituras de disco ficam altas indica que o working set não cabe na memória.',
    },
    transactions: {
      shows: 'Transações com commit e com rollback por segundo, em todos os bancos de dados.',
      look: 'Rollbacks acima de alguns por cento dos commits costumam apontar para erros na aplicação, deadlocks ou timeouts. Uma queda repentina nos commits enquanto as conexões seguem ativas pode ser um travamento, como esperas por lock ou um disco lento. Compare o formato com os gráficos de CPU e de disco para ver qual recurso a carga mais usa.',
    },
    'temporary-files-written': {
      shows:
        'Dados gravados em arquivos temporários por segundo. O PostgreSQL grava esses arquivos quando uma ordenação, um hash ou uma etapa parecida não cabe no work_mem.',
      look: 'Picos causados por alguns relatórios grandes são normais. Gravações temporárias constantes indicam que algumas consultas precisam de mais work_mem, de um índice ou de um plano melhor. Configure log_temp_files para encontrar as consultas que gravam esses arquivos.',
    },
    'wal-and-checkpoint-writes': {
      shows:
        'WAL gerado por segundo, e páginas de dados gravadas pelo checkpointer, pelo background writer e pelos próprios backends.',
      look: 'A maior parte das gravações de páginas deve vir do checkpointer. Gravações dos backends indicam que uma consulta precisou gravar uma página suja antes de reutilizar o buffer, e esperou por essa gravação. Muitas gravações dos backends apontam para um shared_buffers pequeno ou um background writer que fica para trás. Picos grandes de checkpoint podem saturar o disco, então compare com a latência de disco.',
    },
    'replication-lag': {
      shows:
        'Standbys na última amostra, de pg_stat_replication, agrupados por estado e estado de sincronização, com o número de standbys e o maior lag de cada grupo.',
      look: 'Este é um único momento, então compare com capturas anteriores para ver se o lag cresce. Lag crescente indica que um standby, seu disco ou a rede não acompanha. Com lag grande, leituras no standby retornam dados antigos. Com replicação assíncrona, um failover então perde as transações mais recentes.',
    },
    'database-size': {
      shows: 'Tamanho em disco de cada banco de dados no momento da captura.',
      look: 'Compare o total com o espaço livre no sistema de arquivos de dados. Compare com capturas anteriores para obter a taxa de crescimento. Crescimento rápido sem dados novos pode ser bloat, então verifique Tuplas mortas.',
    },
    'largest-tables': {
      shows:
        'As seis maiores tabelas. Cada barra é o tamanho estimado da tabela com seus índices e dados TOAST, a partir das contagens de páginas em pg_class.',
      look: 'As contagens de páginas são atualizadas no VACUUM, no ANALYZE e em alguns comandos DDL, então uma tabela que cresceu desde então aparece menor do que é. Uma tabela que também aparece em Tuplas mortas precisa de ajuste de vacuum. Uma varredura completa dessas tabelas é a que mais custa em I/O.',
    },
    'dead-tuples': {
      shows:
        'As seis tabelas com mais tuplas mortas. Uma tupla morta é uma versão antiga de linha que um UPDATE ou DELETE deixou para trás e que o vacuum ainda não removeu.',
      look: 'Uma proporção alta de tuplas mortas, ou um último autovacuum antigo, indica que o autovacuum está atrasado. A tabela então cresce, e as varreduras leem mais páginas. Transações longas e sessões idle in transaction impedem o vacuum de remover tuplas.',
    },
    'lock-waits': {
      shows: 'Pedidos de lock em espera em cada amostra, por tipo de lock.',
      look: 'Esperas curtas vêm e vão em um servidor ocupado. Uma contagem que fica acima de zero indica que sessões bloqueiam umas às outras. Esperas do tipo relation costumam vir de DDL ou LOCK TABLE. Esperas do tipo transactionid indicam linhas com lock de uma transação longa.',
    },
    'longest-lock-wait': {
      shows: 'O maior tempo que qualquer pedido esperou por um lock em cada amostra, em segundos.',
      look: 'Esperas de mais de alguns segundos bloqueiam usuários. Uma única espera longa pode formar uma fila, porque os pedidos seguintes esperam atrás dela. pg_blocking_pids() encontra a sessão que detém o lock.',
    },
    'locks-held': {
      shows: 'Locks mantidos em cada amostra, por modo de lock.',
      look: 'AccessShareLock vem de leituras simples e é normal. AccessExclusiveLock bloqueia toda leitura e gravação na sua tabela. DDL, VACUUM FULL, TRUNCATE e alguns comandos ALTER TABLE obtêm esse lock. Uma contagem alta e persistente de qualquer modo pode indicar transações longas.',
    },
    'longest-time-in-state': {
      shows:
        'Para cada estado de conexão, o maior tempo que qualquer sessão de cliente ficou nele desde a última mudança de estado, em minutos.',
      look: 'Um tempo longo em active é uma consulta lenta. Um tempo longo em idle in transaction segura locks e bloqueia o vacuum, e idle_in_transaction_session_timeout pode encerrar essas sessões. Tempos longos em idle não causam dano por si só.',
    },
    'wait-events': {
      shows:
        'Backends ativos em cada amostra, pelo tipo de evento de espera que informaram. Sem evento de espera significa que o PostgreSQL não informou nenhum. O backend estava executando em uma CPU ou esperando o sistema operacional agendá-lo.',
      look: 'Esperas Lock indicam que sessões bloquearam umas às outras. Esperas IO são tempo esperando por leituras, gravações ou flushes, então compare com os gráficos de latência de disco. Esperas LWLock apontam para disputa dentro do PostgreSQL, por exemplo no WAL ou no mapeamento de buffers. Muitos backends sem evento de espera em um host ocupado podem indicar fila pela CPU.',
    },
    'deadlocks-and-conflicts': {
      shows: 'Deadlocks, conflitos de recuperação e falhas de checksum por intervalo, em todos os bancos de dados.',
      look: 'Cada deadlock aborta uma transação. A correção fica na aplicação, normalmente obtendo os locks na mesma ordem em todo lugar. Conflitos de recuperação acontecem em standbys. Qualquer falha de checksum indica que há dados corrompidos no disco. Verifique o log do servidor e o armazenamento no mesmo dia.',
    },
    'sessions-ended-abnormally': {
      shows:
        'Sessões por intervalo que terminaram sem uma desconexão limpa. Abandonadas significa que o cliente sumiu. Fatal significa que um erro encerrou a sessão, incluindo timeouts como idle_in_transaction_session_timeout. Terminadas significa que um operador encerrou a sessão, por exemplo com pg_terminate_backend.',
      look: 'Sessões abandonadas apontam para problemas de rede, falhas do cliente ou um pooler que derruba conexões. Sessões Fatal vêm de erros e timeouts, e o log do servidor tem a causa de cada uma. Sessões terminadas correspondem a chamadas de pg_terminate_backend feitas por um operador ou uma ferramenta.',
    },
    'block-i-o-time': {
      shows:
        'Tempo que os backends gastaram lendo e gravando blocos de dados, em milissegundos por segundo. Requer track_io_timing ligado.',
      look: 'Um valor de 1000 significa que um backend, em média, passou todo o tempo em I/O. Tempo de leitura que sobe enquanto a Taxa de acerto do cache cai indica que o working set não cabe na memória. Compare com a latência de disco para distinguir um disco lento de leituras demais.',
    },
    'standby-conflicts': {
      shows:
        'Consultas em um standby que a recuperação cancelou, por intervalo e por causa. Um conflito de snapshot acontece quando o vacuum no primário removeu linhas de que uma consulta no standby ainda precisava.',
      look: 'Qualquer valor acima de zero indica que usuários do standby viram erros. Conflitos de snapshot são os mais comuns. hot_standby_feedback ou um max_standby_streaming_delay maior reduzem esses conflitos, ao custo de mais bloat no primário ou mais lag no standby.',
    },
    'wal-archiving': {
      shows: 'Arquivos WAL arquivados, e falhas de arquivamento, por intervalo, de pg_stat_archiver.',
      look: 'Qualquer falha indica que o arquivamento está atrasado, e a recuperação point in time não consegue passar do último arquivo arquivado. O PostgreSQL mantém todo arquivo WAL que não conseguiu arquivar, então o pg_wal cresce até o comando de arquivamento voltar a funcionar e pode encher o disco. Os arquivos arquivados devem acompanhar o WAL gerado.',
    },
    'replication-slots': {
      shows:
        'WAL que o PostgreSQL mantém em disco para slots de replicação, como a maior quantidade retida por qualquer slot de cada tipo, físico ou lógico.',
      look: 'Um slot mantém o WAL até seu consumidor confirmá-lo. Uma linha que só cresce indica que um consumidor está lento, parado ou não existe mais. Sem max_slot_wal_keep_size, esse WAL pode encher o disco e parar o servidor. Remova os slots que ninguém usa.',
    },
    'slru-reads': {
      shows:
        'Blocos lidos do disco por segundo pelos caches SLRU. Esses caches pequenos guardam o status das transações, subtransações, multixacts e timestamps de commit.',
      look: 'Valores perto de zero são normais. Leituras constantes de subtransações vêm do uso intenso de savepoints ou de blocos de exceção em PL/pgSQL, e podem deixar o servidor inteiro lento. Leituras de multixact vêm de muitas sessões aplicando lock nas mesmas linhas, por exemplo com SELECT FOR SHARE ou chaves estrangeiras.',
    },
    'i-o-by-operation': {
      shows:
        'Dados lidos, gravados e estendidos por segundo por todos os backends, de pg_stat_io. Extensões são blocos novos adicionados ao fim de um arquivo de tabela ou de índice.',
      look: 'Leituras grandes apontam para varreduras de tabelas que não cabem na memória. As gravações devem acompanhar os checkpoints e o background writer, então compare com Gravações de WAL e checkpoint. Extensões constantes são normais em tabelas que crescem.',
    },
    rows: {
      shows:
        'Linhas por segundo em todos os bancos de dados. Leitura são as linhas retornadas por varreduras. Gravação são as linhas inseridas, atualizadas e excluídas.',
      look: 'Leitura conta toda linha que uma varredura retornou, antes dos filtros, então uma taxa de leitura alta com poucas transações aponta para varreduras sequenciais de tabelas grandes. Uma taxa de gravação que sobe junto com o WAL gerado é normal. Compare o formato com Transações por segundo.',
    },
  },
}

export default translation
