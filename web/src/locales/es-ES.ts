import type { Translation } from '../i18n.ts'

const translation: Translation = {
  messages: {
    ', every sample': ', en todas las muestras',
    '1 minute': '1 minuto',
    '1 sample': '1 muestra',
    '1 source failed': '1 fuente falló',
    '1 source is not available on this host or server version':
      '1 fuente no está disponible en este host o en esta versión del servidor',
    '15 minutes': '15 minutos',
    '5 minutes': '5 minutos',
    Abandoned: 'Abandonadas',
    'Active backends, by wait event type': 'Backends activos, por tipo de evento de espera',
    'All databases': 'Todas las bases de datos',
    'All {total} sources': 'Las {total} fuentes',
    Archived: 'Archivados',
    'Artifacts do not record this today, so there is nothing to show for this snapshot.':
      'Los artefactos todavía no registran este dato, así que no hay nada que mostrar en este snapshot.',
    'Average read latency, five busiest disks (ms)':
      'Latencia media de lectura, cinco discos más ocupados (ms)',
    'Average requests in flight, five busiest disks':
      'Media de peticiones en curso, cinco discos más ocupados',
    'Average write latency, five busiest disks (ms)':
      'Latencia media de escritura, cinco discos más ocupados (ms)',
    'Backend writes': 'Escrituras de backends',
    'Background writer': 'Background writer',
    'Block I/O time': 'Tiempo de I/O de bloques',
    'Blocked on I/O': 'Bloqueados en I/O',
    'Buffer pin': 'Buffer pin',
    Buffers: 'Buffers',
    CPU: 'CPU',
    'CPU busy': 'CPU ocupada',
    'CPU utilization': 'Uso de CPU',
    'CPU utilization by state (%)': 'Uso de CPU por estado (%)',
    CPUs: 'CPU',
    'Cache hit ratio': 'Tasa de aciertos de caché',
    'Cache hit ratio per interval (%)': 'Tasa de aciertos de caché por intervalo (%)',
    Capture: 'Captura',
    'Cgroup I/O': 'I/O del cgroup',
    'Cgroup I/O, all devices (MiB/s)': 'I/O del cgroup, todos los dispositivos (MiB/s)',
    'Cgroup events': 'Eventos del cgroup',
    'Cgroup limit events per interval': 'Eventos de límite del cgroup por intervalo',
    'Cgroup memory': 'Memoria del cgroup',
    'Charts cover': 'Periodo de los gráficos',
    'Checkpoint writes': 'Escrituras del checkpoint',
    'Checksum failures': 'Fallos de checksum',
    'Client connections by state': 'Conexiones de clientes por estado',
    'Cluster size': 'Tamaño del clúster',
    'Collected in full.': 'Recopiladas por completo.',
    'Collected in part': 'Recopiladas en parte',
    'Collected, {count} of them in part. See page 2.':
      'Recopiladas, {count} de ellas en parte. Consulte la página 2.',
    'Collected. See page 2.': 'Recopiladas. Consulte la página 2.',
    Commits: 'Commits',
    Complete: 'Completa',
    'Connected as': 'Conectado como',
    Connections: 'Conexiones',
    'Connections by state': 'Conexiones por estado',
    Contents: 'Índice',
    Current: 'Actual',
    'Data coverage': 'Cobertura de los datos',
    'Database size': 'Tamaño de las bases de datos',
    'Dead (%)': 'Muertas (%)',
    'Dead tuples': 'Tuplas muertas',
    Deadlock: 'Deadlock',
    Deadlocks: 'Deadlocks',
    'Deadlocks and conflicts': 'Deadlocks y conflictos',
    'Deadlocks and recovery conflicts per interval':
      'Deadlocks y conflictos de recuperación por intervalo',
    'Deadlocks, recovery conflicts and checksum failures per interval':
      'Deadlocks, conflictos de recuperación y fallos de checksum por intervalo',
    'Direct, by the allocating process': 'Directa, por el proceso que reserva memoria',
    'Disk busy': 'Disco ocupado',
    'Disk busy time, five busiest disks (%)': 'Tiempo ocupado de disco, cinco discos más ocupados (%)',
    'Disk queue depth': 'Profundidad de cola de disco',
    'Disk read latency': 'Latencia de lectura de disco',
    'Disk throughput': 'Transferencia de disco',
    'Disk throughput, all disks (MiB/s)': 'Transferencia de disco, todos los discos (MiB/s)',
    'Disk utilization': 'Utilización de disco',
    'Disk write latency': 'Latencia de escritura de disco',
    Duration: 'Duración',
    'Each value is the last sample, or the last interval for rates. Peaks cover the whole session.':
      'Cada valor es la última muestra, o el último intervalo en el caso de las tasas. Los picos cubren toda la captura.',
    Endpoint: 'Endpoint',
    Error: 'Error',
    Extends: 'Extends',
    Failed: 'Fallos',
    Fatal: 'Fatal',
    'Filesystem usage': 'Uso del sistema de ficheros',
    Free: 'Libre',
    Host: 'Host',
    'I/O': 'I/O',
    'I/O by operation': 'I/O por operación',
    'I/O by operation, all backends (MiB/s)': 'I/O por operación, todos los backends (MiB/s)',
    'I/O wait': 'Espera de I/O',
    Lock: 'Bloqueo',
    'It also needs track_io_timing on the server.': 'También necesita track_io_timing activado en el servidor.',
    Kernel: 'Kernel',
    Killed: 'Terminadas',
    'Largest tables': 'Tablas más grandes',
    'Largest {type} slot': 'Slot {type} más grande',
    'Last autovacuum': 'Último autovacuum',
    'Live tuples': 'Tuplas vivas',
    'Load average': 'Carga media',
    'Load average, 1 / 5 / 15 minutes': 'Carga media, 1 / 5 / 15 minutos',
    'Load, 1 minute': 'Carga, 1 minuto',
    'Lock requests waiting, by lock type': 'Solicitudes de bloqueo en espera, por tipo de bloqueo',
    'Lock waits': 'Esperas de bloqueo',
    'Locks held': 'Bloqueos mantenidos',
    'Locks held, by mode': 'Bloqueos mantenidos, por modo',
    Logs: 'Logs',
    'Longest lock wait': 'Espera de bloqueo más larga',
    'Longest lock wait (s)': 'Espera de bloqueo más larga (s)',
    'Longest time in state': 'Mayor tiempo en un estado',
    'Longest time in state since state_change (min)': 'Mayor tiempo en un estado desde state_change (min)',
    'Longest wait': 'Espera más larga',
    'Lowest {value} at {time}': 'Mínimo de {value} a las {time}',
    'Major faults': 'Fallos mayores',
    'Major page faults': 'Fallos de página mayores',
    'Major page faults per second': 'Fallos de página mayores por segundo',
    'Max lag': 'Retraso máximo',
    Memory: 'Memoria',
    'Memory and swap charged to the cgroup (GiB)': 'Memoria y swap imputadas al cgroup (GiB)',
    'Memory by use, up to MemTotal (GiB)': 'Memoria por uso, hasta MemTotal (GiB)',
    'Memory used': 'Memoria usada',
    Metrics: 'Métricas',
    'Most expensive statements': 'Sentencias más costosas',
    'Needs a new postgres.stats.statements collector, after the decision on whether query text may enter the artifact.':
      'Necesita un nuevo colector postgres.stats.statements, después de decidir si el texto de las consultas puede entrar en el artefacto.',
    'Network drops and errors': 'Descartes y errores de red',
    'Network throughput': 'Transferencia de red',
    'Network throughput, all interfaces (MiB/s)': 'Transferencia de red, todas las interfaces (MiB/s)',
    'Nice, IRQ, softirq': 'Nice, IRQ, softirq',
    'No collected samples in this window.': 'No hay muestras recopiladas en esta ventana.',
    'No dead tuples in the recorded tables.': 'No hay tuplas muertas en las tablas registradas.',
    'No measurements.': 'Sin mediciones.',
    'No standby connected': 'Ningún standby conectado',
    'No standby was connected in the last sample.': 'No había ningún standby conectado en la última muestra.',
    None: 'Ninguno',
    'Not available on this host or version': 'No disponibles en este host o en esta versión',
    'Not collected': 'No recopilado',
    'Not recorded in this artifact.': 'No registrado en este artefacto.',
    'Not recorded in this artifact. Captures before schema version 3 have no mount sizes.':
      'No registrado en este artefacto. Las capturas anteriores a la versión 3 del esquema no tienen el tamaño de los puntos de montaje.',
    'Not recorded in this artifact. Captures before schema version 3 have no network counters.':
      'No registrado en este artefacto. Las capturas anteriores a la versión 3 del esquema no tienen contadores de red.',
    'Not recorded in this artifact. Captures before schema version 3 have no relations.':
      'No registrado en este artefacto. Las capturas anteriores a la versión 3 del esquema no tienen relaciones.',
    'Not recorded in this artifact: proc.pressure is {reason}. Older kernels and some containers have no PSI.':
      'No registrado en este artefacto: el estado de proc.pressure es {reason}. Los kernels antiguos y algunos contenedores no tienen PSI.',
    'Not recorded in this artifact: {reason}.': 'No registrado en este artefacto: {reason}.',
    Other: 'Otros',
    'Other states': 'Otros estados',
    Overview: 'Visión general',
    'Packets dropped and errors per second, all interfaces':
      'Paquetes descartados y errores por segundo, todas las interfaces',
    'Page cache': 'Caché de páginas',
    'Page reclaim': 'Recuperación de páginas',
    'Pages scanned for reclaim per second': 'Páginas escaneadas para recuperación por segundo',
    Partial: 'Parcial',
    'Peak {value} at {time}': 'Pico de {value} a las {time}',
    PostgreSQL: 'PostgreSQL',
    'PostgreSQL capture report': 'Informe de captura de PostgreSQL',
    'PostgreSQL panels': 'Paneles de PostgreSQL',
    'PostgreSQL {version} does not have these statistics':
      'PostgreSQL {version} no tiene estas estadísticas',
    'Pressure stall': 'Presión (PSI)',
    'Pressure stall, some tasks, 10 s average (%)': 'Presión (PSI), algunas tareas, media de 10 s (%)',
    Processes: 'Procesos',
    'Processes running and blocked': 'Procesos en ejecución y bloqueados',
    Queries: 'Consultas',
    'Queries canceled by recovery conflicts per interval':
      'Consultas canceladas por conflictos de recuperación, por intervalo',
    'Query latency': 'Latencia de las consultas',
    'Query panels': 'Paneles de consultas',
    'Query source': 'Fuente de datos de consultas',
    'Query volume': 'Volumen de consultas',
    Read: 'Lectura',
    Reads: 'Lecturas',
    Reason: 'Motivo',
    Received: 'Recibidos',
    'Received, dropped': 'Recibidos, descartados',
    'Received, errors': 'Recibidos, errores',
    'Recovery conflicts': 'Conflictos de recuperación',
    'Replica lag': 'Retraso de la réplica',
    'Replication lag': 'Retraso de replicación',
    'Replication slots': 'Slots de replicación',
    Rollbacks: 'Rollbacks',
    Rows: 'Filas',
    'Rows per second': 'Filas por segundo',
    Running: 'En ejecución',
    'SLRU blocks read from disk per second': 'Bloques SLRU leídos de disco por segundo',
    'SLRU reads': 'Lecturas de SLRU',
    Samples: 'Muestras',
    Sent: 'Enviados',
    'Sent, dropped': 'Enviados, descartados',
    'Sent, errors': 'Enviados, errores',
    Series: 'Serie',
    'Sessions ended abnormally': 'Sesiones finalizadas de forma anómala',
    'Sessions ended abnormally per interval': 'Sesiones finalizadas de forma anómala por intervalo',
    'Shared memory': 'Memoria compartida',
    'Size per database': 'Tamaño por base de datos',
    'Size per table (GiB)': 'Tamaño por tabla (GiB)',
    Snapshot: 'Snapshot',
    'Snapshot sections': 'Secciones del snapshot',
    'Some databases do not accept connections': 'Algunas bases de datos no aceptan conexiones',
    'Some databases were not visited: {reason}.': 'Algunas bases de datos no se visitaron: {reason}.',
    'Some of the data came in. The rest is expected to be missing.':
      'Se recopiló parte de los datos. Es normal que falte el resto.',
    Source: 'Fuente',
    Sources: 'Fuentes',
    'Standby conflicts': 'Conflictos en standby',
    Standbys: 'Standbys',
    State: 'Estado',
    Status: 'Estado',
    Steal: 'Steal',
    Swap: 'Swap',
    'Swap activity': 'Actividad de swap',
    'Swap in use': 'Swap en uso',
    'Swap used (GiB)': 'Swap en uso (GiB)',
    'Swapped in': 'Leído de swap',
    'Swapped in and out (MiB/s)': 'Lectura y escritura de swap (MiB/s)',
    'Swapped out': 'Escrito en swap',
    'Sync state': 'Estado de sincronización',
    System: 'Sistema',
    'System panels': 'Paneles del sistema',
    Table: 'Tabla',
    Tablespace: 'Tablespace',
    'Temporary files written': 'Ficheros temporales escritos',
    'Temporary files written (MiB/s)': 'Ficheros temporales escritos (MiB/s)',
    'The PostgreSQL cgroup has no I/O statistics':
      'El cgroup de PostgreSQL no tiene estadísticas de I/O',
    'The PostgreSQL cgroup has no events file':
      'El cgroup de PostgreSQL no tiene el fichero de eventos',
    'The capture ran to the end.': 'La captura llegó hasta el final.',
    'The capture stopped early, or a source failed.':
      'La captura se detuvo antes del final, o una fuente falló.',
    'The host has no /etc/os-release': 'El host no tiene /etc/os-release',
    'The kernel has no pressure stall information (PSI)':
      'El kernel no ofrece pressure stall information (PSI)',
    'The kernel has no transparent huge pages settings':
      'El kernel no tiene los ajustes de transparent huge pages',
    'The source exists on this host, but pg-probe could not read it. Fix the cause and capture again.':
      'La fuente existe en este host, pero pg-probe no pudo leerla. Corrija la causa y vuelva a capturar.',
    'The whole session': 'Toda la captura',
    'These databases do not accept connections: {names}':
      'Estas bases de datos no aceptan conexiones: {names}',
    'These sources have no data in this artifact. Panels that read them show an unknown value.':
      'Estas fuentes no tienen datos en este artefacto. Los paneles que dependen de ellas muestran un valor desconocido.',
    'This PostgreSQL version does not have these statistics':
      'Esta versión de PostgreSQL no tiene estas estadísticas',
    'This kernel or server version does not have these sources. Nothing to fix.':
      'Este kernel o esta versión del servidor no tiene estas fuentes. No hay nada que corregir.',
    'Time reading and writing blocks (ms per second)': 'Tiempo leyendo y escribiendo bloques (ms por segundo)',
    'Time split': 'Reparto del tiempo',
    Timestamp: 'Fecha y hora',
    Tool: 'Herramienta',
    Transactions: 'Transacciones',
    'Transactions per second': 'Transacciones por segundo',
    'Unknown: {reason}.': 'Desconocido: {reason}.',
    'Unknown, {reason}': 'Desconocido, {reason}',
    'Not waiting': 'Sin espera',
    Used: 'En uso',
    User: 'Usuario',
    Version: 'Versión',
    'View exact values': 'Ver valores exactos',
    'WAL and buffer writes (MiB/s)': 'Escrituras de WAL y de buffers (MiB/s)',
    'WAL and checkpoint writes': 'Escrituras de WAL y checkpoint',
    'WAL archiving': 'Archivado de WAL',
    'WAL files archived per interval': 'Ficheros WAL archivados por intervalo',
    'WAL generated': 'WAL generado',
    'WAL retained by replication slots (MiB)': 'WAL retenido por slots de replicación (MiB)',
    'Wait events': 'Eventos de espera',
    Waiting: 'En espera',
    'What to look for': 'Qué observar',
    Why: 'Motivo',
    'Worst standby': 'Peor standby',
    Write: 'Escritura',
    Writes: 'Escrituras',
    Written: 'Escritura',
    'a rate needs two or more samples, and this artifact holds one':
      'una tasa necesita dos o más muestras, y este artefacto solo tiene una',
    'all interfaces': 'todas las interfaces',
    blocked: 'bloqueado',
    'by total time in the window': 'por tiempo total en la ventana',
    'calls per second': 'llamadas por segundo',
    'captured {time}': 'capturado el {time}',
    'captured {time} in {duration}': 'capturado el {time}, con una duración de {duration}',
    'every sample': 'todas las muestras',
    failed: 'falló',
    'heap, indexes and TOAST, top 6': 'heap, índices y TOAST, top 6',
    'kswapd, in the background': 'kswapd, en segundo plano',
    'last sample, from pg_stat_replication': 'última muestra, de pg_stat_replication',
    'no collector': 'sin colector',
    'non-I/O against block I/O': 'tiempo sin I/O frente a I/O de bloques',
    'not recorded': 'no registrado',
    'not recorded in this artifact': 'no registrado en este artefacto',
    'of {max} max_connections': 'de {max} en max_connections',
    'on {distribution}': 'en {distribution}',
    'percentiles across the window': 'percentiles en la ventana',
    permission_denied: 'permiso denegado',
    'pg-probe read {total} sources. {full} collected in full. Charts show the rest as unknown, never as zero.':
      'pg-probe leyó {total} fuentes. {full} se recopilaron por completo. Los gráficos muestran el resto como desconocido, nunca como cero.',
    'pg_stat_statements is installed in {databases}. The artifact does not read it yet.':
      'pg_stat_statements está instalado en {databases}. El artefacto todavía no lee esos datos.',
    'pg_stat_statements is not installed in any database the probe visited.':
      'pg_stat_statements no está instalado en ninguna base de datos que visitó pg-probe.',
    'pg_stat_statements stores totals and means, never a distribution. Percentiles need the server log, which the probe does not read.':
      'pg_stat_statements guarda totales y medias, nunca una distribución. Los percentiles necesitan el log del servidor, que pg-probe no lee.',
    'point in time': 'momento de la captura',
    'point in time, local disks': 'momento de la captura, discos locales',
    primary: 'primario',
    'since the last statistics reset': 'desde el último reset de estadísticas',
    'some, 10s window': 'some, ventana de 10s',
    standby: 'standby',
    'stated, never guessed': 'declarado, nunca supuesto',
    unsupported: 'no admitido',
    'vacuum backlog, top 6': 'vacuum pendiente, top 6',
    '{collected} of {total} sources': '{collected} de {total} fuentes',
    '{count} CPUs online': '{count} CPU en línea',
    '{count} of {total}': '{count} de {total}',
    '{count} samples': '{count} muestras',
    '{count} samples, every {step} s': '{count} muestras, cada {step} s',
    '{count} sources are not available on this host or server version':
      '{count} fuentes no están disponibles en este host o en esta versión del servidor',
    '{count} sources failed': '{count} fuentes fallaron',
    '{date}, {from} to {to} UTC': '{date}, de {from} a {to} UTC',
    '{disk}, the busiest disk': '{disk}, el disco más ocupado',
    '{from} to {to} UTC': 'de {from} a {to} UTC',
    '{full} of {total} collected in full. See page 2.':
      '{full} de {total} recopiladas por completo. Consulte la página 2.',
    '{label}: open the {chart} chart': '{label}: abrir el gráfico {chart}',
    '{label}: summary': '{label}: resumen',
    '{used} of {total}': '{used} de {total}',
    '{used} of {total} {unit}': '{used} de {total} {unit}',
    '{user}, superuser': '{user}, superusuario',
    '{what} are not collected yet': '{what} todavía no se recopilan',
  },
  explain: {
    'cpu-utilization': {
      shows:
        'Proporción del tiempo de CPU en cada estado, como media de todas las CPU. Usuario es el tiempo en programas como los backends de PostgreSQL, y Sistema es el tiempo en el kernel. Espera de I/O es tiempo ocioso mientras había una petición de disco pendiente. Steal es el tiempo que el hipervisor dio a otras máquinas virtuales.',
      look: 'Sume Usuario y Sistema para obtener el tiempo ocupado. Cuando el tiempo ocupado se mantiene por encima del 80% durante minutos, revise la Carga media. Una carga por encima del número de CPU en el mismo periodo puede indicar que las consultas esperaron un núcleo, pero la carga de Linux también cuenta las tareas que esperan al disco. Un p95 cerca del 100% con un p50 bajo en la tabla indica ráfagas cortas, no una falta constante. Una Espera de I/O por encima del 10% indica que las CPU estuvieron ociosas mientras había peticiones de disco pendientes, así que revise los gráficos de latencia. Un Steal por encima de un pequeño porcentaje indica que la instancia comparte sus CPU con vecinos ocupados.',
    },
    'load-average': {
      shows:
        'Número medio de tareas en ejecución o esperando para ejecutarse, en los últimos 1, 5 y 15 minutos. En Linux, la cuenta también incluye las tareas que esperan al disco.',
      look: 'Compare la carga con el número de CPU en el indicador Carga, 1 minuto, de la Visión general. Una carga por encima del número de CPU indica que las tareas esperaron un núcleo o un disco. Una línea de 1 minuto muy por encima de la de 15 minutos es un pico que acaba de empezar. Una carga alta con poco tiempo ocupado de CPU indica que las tareas esperan al disco, no a la CPU.',
    },
    memory: {
      shows:
        'Cómo usó el host su RAM. En uso es la memoria que ocupan los programas. Memoria compartida incluye el shared_buffers de PostgreSQL. Caché de páginas son datos de ficheros que el kernel guarda para lecturas rápidas, y Buffers son metadatos del sistema de ficheros. Libre es la memoria que nadie usa.',
      look: 'Poca memoria Libre, por sí sola, es normal, porque Linux llena la memoria sobrante con caché de páginas. La señal de alarma es una caché de páginas que se reduce mientras En uso crece. Entonces PostgreSQL encuentra menos datos en memoria y lee más del disco. Lea este gráfico junto con Tasa de aciertos de caché y Swap en uso.',
    },
    'swap-in-use': {
      shows: 'Espacio de swap en uso en el host.',
      look: 'Una línea plana suele ser datos antiguos que el kernel movió hace tiempo, y no hace daño. Una línea que sube durante la captura indica que al host le faltó memoria mientras se capturaban los datos. Lectura y escritura de swap muestra si el host vuelve a leer esos datos, y eso es lo que ralentiza las consultas.',
    },
    'disk-throughput': {
      shows: 'Bytes leídos y escritos por segundo en todos los discos.',
      look: 'Una línea que se queda plana en el mismo valor máximo durante minutos suele ser un límite del dispositivo o del volumen en la nube. Un pico de lectura que coincide con una caída de la Tasa de aciertos de caché indica que las consultas leyeron datos que no estaban en memoria. Los picos de escritura cada pocos minutos suelen ser checkpoints. Compare con el Tiempo ocupado de disco y con los gráficos de latencia.',
    },
    'disk-utilization': {
      shows:
        'Proporción del tiempo en que cada uno de los cinco discos más ocupados tuvo al menos una petición en curso.',
      look: 'En un único HDD, valores cerca del 100% indican que el disco está saturado. Los SSD, los arrays RAID y los volúmenes en la nube atienden muchas peticiones a la vez, así que pueden mostrar el 100% y aún tener margen. En esos casos, fíese más de la latencia de lectura y escritura que de este gráfico.',
    },
    'pressure-stall': {
      shows:
        'Proporción del tiempo en que al menos una tarea se quedó parada esperando CPU, memoria o I/O, en ventanas de 10 segundos. El kernel lo informa en /proc/pressure.',
      look: 'Los valores cerca de cero son sanos. Un valor que se mantiene por encima del 10% muestra qué recurso frenó el trabajo. Una presión de memoria por encima de cero indica que las tareas esperaron mientras el kernel liberaba memoria. La presión de I/O suele coincidir con una latencia de disco alta.',
    },
    'filesystem-usage': {
      shows: 'Espacio usado en cada sistema de ficheros local en el momento de la captura.',
      look: 'Por encima del 85% en el sistema de ficheros de datos o de WAL queda poco margen para el crecimiento, los ficheros temporales y las ráfagas de WAL. Si el sistema de ficheros del WAL se llena, PostgreSQL se detiene.',
    },
    'network-throughput': {
      shows: 'Bytes recibidos y enviados por segundo, sumando todas las interfaces.',
      look: 'Un techo plano cerca de la velocidad del enlace indica que la red es el límite. Un enlace de 1 Gbit/s llega a unos 119 MiB/s. Los picos repentinos suelen coincidir con copias de seguridad, un standby que se pone al día o consultas que devuelven resultados grandes. El tráfico enviado por un primario crece con cada standby.',
    },
    processes: {
      shows:
        'Procesos en ejecución en una CPU, y procesos bloqueados, que en Linux casi siempre significa que esperan al disco.',
      look: 'En ejecución por encima del número de CPU indica que las tareas esperaron un núcleo. Bloqueados en I/O por encima de cero durante varias muestras seguidas indica que las tareas esperaron al almacenamiento. Compare con Espera de I/O en Uso de CPU y con los gráficos de latencia de disco.',
    },
    'swap-activity': {
      shows: 'Datos que el kernel movió de la RAM al swap, y del swap de vuelta a la RAM, por segundo.',
      look: 'Leído de swap es la línea que hace daño, porque un proceso se detiene hasta que cada página vuelve del disco. Cualquier lectura constante de swap durante la captura ralentiza PostgreSQL. Escrito en swap, por sí solo, indica que el kernel está haciendo sitio, a menudo para la caché de páginas.',
    },
    'major-page-faults': {
      shows: 'Fallos de página por segundo que necesitaron una lectura de disco, porque la página no estaba en memoria.',
      look: 'Una tasa baja y estable es normal cuando los programas arrancan o leen ficheros mapeados en memoria. Los picos que coinciden con lecturas de swap o con una caché de páginas que se reduce indican que al host le faltó memoria.',
    },
    'page-reclaim': {
      shows:
        'Páginas que el kernel escaneó por segundo para liberar memoria. kswapd escanea en segundo plano. La recuperación directa ocurre cuando un proceso necesita memoria y tiene que liberar parte por sí mismo antes de continuar.',
      look: 'Algo de actividad de kswapd es normal en un host ocupado. Una recuperación directa por encima de cero indica que los procesos, incluidos los backends de PostgreSQL, se quedaron parados mientras pedían memoria. Eso provoca picos de latencia aunque el swap se mantenga estable.',
    },
    'network-drops-and-errors': {
      shows: 'Paquetes descartados y errores de interfaz por segundo, recibidos y enviados, en todas las interfaces.',
      look: 'Un host sano muestra cero. Los descartes constantes en recepción suelen indicar que los buffers de la interfaz se desbordan bajo carga. Los errores apuntan a un problema de cable, driver o tarjeta de red. Cada descarte hace que TCP reenvíe los datos, lo que ralentiza a los clientes y a los standbys.',
    },
    'disk-read-latency': {
      shows: 'Tiempo medio de una petición de lectura en cada uno de los cinco discos más ocupados, en milisegundos.',
      look: 'Los SSD locales suelen responder en menos de 1 ms, los volúmenes en la nube en 1 a 5 ms y los HDD en 5 a 15 ms. Valores de decenas de ms que se mantienen indican que el disco estaba sobrecargado o limitado. El p95 de la tabla muestra lo lentos que fueron los peores intervalos.',
    },
    'disk-write-latency': {
      shows: 'Tiempo medio de una petición de escritura en cada uno de los cinco discos más ocupados, en milisegundos.',
      look: 'Con synchronous_commit activado, cada commit espera a que su WAL llegue al disco. Por eso, una latencia de escritura alta en el disco del WAL ralentiza todas las transacciones. En SSD o almacenamiento en la nube, escrituras por encima de 10 ms hacen que los commits sean tan lentos que los usuarios lo notan. Los picos cada pocos minutos suelen coincidir con checkpoints.',
    },
    'disk-queue-depth': {
      shows: 'Número medio de peticiones en curso en cada uno de los cinco discos más ocupados.',
      look: 'Una cola que crece junto con la latencia indica que el disco no daba abasto. Una cola profunda con latencia baja no es un problema, porque los SSD y los volúmenes en la nube funcionan mejor con muchas peticiones a la vez.',
    },
    'cgroup-memory': {
      shows:
        'Memoria y swap imputadas al cgroup de pg-probe. Cuando pg-probe se ejecuta en el contenedor o en la unidad systemd de PostgreSQL, es la memoria de ese contenedor.',
      look: 'Cerca del límite, el kernel recupera memoria dentro del cgroup. En el límite, mata un proceso. La caché de páginas también cuenta para el límite, así que un cgroup puede parecer lleno cuando casi todo es caché.',
    },
    'cgroup-i-o': {
      shows: 'Bytes leídos y escritos por segundo por el cgroup de pg-probe, en todos los dispositivos.',
      look: 'Un techo plano puede ser un límite de I/O definido en el contenedor. Si la Transferencia de disco de todo el host tiene margen mientras esta línea está plana, el límite está en el contenedor, no en el disco.',
    },
    'cgroup-events': {
      shows:
        'Cambio por intervalo de hasta cinco contadores del cgroup de pg-probe, con nombre en el formato controlador.contador, de cpu.stat, memory.events y pids.events. Los contadores de CPU que terminan en usec son microsegundos de tiempo de CPU. Los demás contadores cuentan eventos.',
      look: 'cpu.nr_throttled por encima de cero indica que el límite de CPU del cgroup pausó sus tareas. memory.high cuenta las veces que el kernel frenó el cgroup y recuperó memoria en su límite flexible, lo que nunca mata un proceso. memory.max cuenta las veces que el cgroup llegó a su límite estricto. El kernel solo mata un proceso cuando la recuperación falla en ese punto, y memory.oom_kill cuenta esas muertes. La muerte de cualquier proceso de PostgreSQL reinicia el servidor. pids.max indica que el cgroup no pudo iniciar un proceso, así que las conexiones nuevas fallan.',
    },
    'connections-by-state': {
      shows:
        'Conexiones de clientes en cada muestra, por estado. Las conexiones active ejecutan una consulta. Las conexiones idle esperan al cliente. Las conexiones idle in transaction tienen una transacción abierta, pero no ejecutan nada.',
      look: 'Muchas más conexiones active que CPU indican que las consultas compiten por los núcleos. Las conexiones idle in transaction que se prolongan mantienen bloqueos e impiden que el vacuum elimine tuplas muertas. Compare el total con max_connections en el indicador Conexiones. Muchas conexiones idle sugieren usar un pooler de conexiones.',
    },
    'cache-hit-ratio': {
      shows:
        'Proporción de lecturas de bloques que PostgreSQL encontró en shared_buffers, por intervalo, en todas las bases de datos. La caché de páginas de Linux aún puede servir un fallo de caché, así que un fallo no siempre es una lectura de disco.',
      look: 'Las cargas OLTP suelen mantenerse por encima del 99%. Las caídas coinciden con escaneos de tablas grandes, informes o una caché fría tras un reinicio. Una tasa que se mantiene baja mientras las lecturas de disco siguen altas indica que el working set no cabe en memoria.',
    },
    transactions: {
      shows: 'Transacciones confirmadas con commit y deshechas con rollback por segundo, en todas las bases de datos.',
      look: 'Unos rollbacks por encima de un pequeño porcentaje de los commits suelen apuntar a errores de la aplicación, deadlocks o timeouts. Una caída repentina de los commits mientras las conexiones siguen activas puede ser un atasco, como esperas de bloqueo o un disco lento. Compare la forma con los gráficos de CPU y de disco para ver qué recurso usa más la carga.',
    },
    'temporary-files-written': {
      shows:
        'Datos escritos en ficheros temporales por segundo. PostgreSQL los escribe cuando una ordenación, un hash o un paso similar no cabe en work_mem.',
      look: 'Los picos de unos pocos informes grandes son normales. Las escrituras temporales constantes indican que algunas consultas necesitan más work_mem, un índice o un plan mejor. Configure log_temp_files para encontrar las consultas que los escriben.',
    },
    'wal-and-checkpoint-writes': {
      shows:
        'WAL generado por segundo, y páginas de datos escritas por el checkpointer, el background writer y los propios backends.',
      look: 'La mayoría de las escrituras de páginas debería venir del checkpointer. Las escrituras de backends indican que una consulta tuvo que escribir una página sucia antes de reutilizar el buffer, y esperó a esa escritura. Muchas escrituras de backends apuntan a un shared_buffers pequeño o a un background writer que se queda atrás. Los picos grandes de checkpoint pueden saturar el disco, así que compare con la latencia de disco.',
    },
    'replication-lag': {
      shows:
        'Standbys en la última muestra, de pg_stat_replication, agrupados por estado y estado de sincronización, con el número de standbys y el mayor retraso de cada grupo.',
      look: 'Es un único momento, así que compare con capturas anteriores para ver si el retraso crece. Un retraso que crece indica que un standby, su disco o la red no dan abasto. Con un retraso grande, las lecturas en el standby devuelven datos antiguos. Con replicación asíncrona, un failover pierde entonces las transacciones más recientes.',
    },
    'database-size': {
      shows: 'Tamaño en disco de cada base de datos en el momento de la captura.',
      look: 'Compare el total con el espacio libre en el sistema de ficheros de datos. Compare con capturas anteriores para obtener el ritmo de crecimiento. Un crecimiento rápido sin datos nuevos puede ser bloat, así que revise Tuplas muertas.',
    },
    'largest-tables': {
      shows:
        'Las seis tablas más grandes. Cada barra es el tamaño estimado de la tabla con sus índices y datos TOAST, a partir del número de páginas en pg_class.',
      look: 'El número de páginas se actualiza con VACUUM, ANALYZE y algunas sentencias DDL, así que una tabla que ha crecido desde entonces aparece más pequeña de lo que es. Una tabla que también aparece en Tuplas muertas necesita ajustar el vacuum. Un escaneo completo de estas tablas es el que más I/O cuesta.',
    },
    'dead-tuples': {
      shows:
        'Las seis tablas con más tuplas muertas. Una tupla muerta es una versión antigua de una fila que un UPDATE o un DELETE dejó atrás y que el vacuum aún no ha eliminado.',
      look: 'Una proporción alta de tuplas muertas, o un último autovacuum lejano, indica que el autovacuum va con retraso. Entonces la tabla crece y los escaneos leen más páginas. Las transacciones largas y las sesiones idle in transaction impiden que el vacuum elimine tuplas.',
    },
    'lock-waits': {
      shows: 'Solicitudes de bloqueo en espera en cada muestra, por tipo de bloqueo.',
      look: 'Las esperas cortas van y vienen en un servidor ocupado. Un recuento que se mantiene por encima de cero indica que las sesiones se bloquean entre sí. Las esperas de tipo relation suelen venir de DDL o de LOCK TABLE. Las esperas de tipo transactionid indican filas bloqueadas por una transacción larga.',
    },
    'longest-lock-wait': {
      shows: 'El mayor tiempo que cualquier solicitud llevaba esperando un bloqueo en cada muestra, en segundos.',
      look: 'Las esperas de más de unos segundos bloquean a los usuarios. Una sola espera larga puede formar una cola, porque las solicitudes posteriores esperan detrás de ella. pg_blocking_pids() encuentra la sesión que tiene el bloqueo.',
    },
    'locks-held': {
      shows: 'Bloqueos mantenidos en cada muestra, por modo de bloqueo.',
      look: 'AccessShareLock viene de lecturas simples y es normal. AccessExclusiveLock bloquea toda lectura y escritura en su tabla. Lo toman DDL, VACUUM FULL, TRUNCATE y algunos comandos ALTER TABLE. Un recuento alto y sostenido de cualquier modo puede indicar transacciones largas.',
    },
    'longest-time-in-state': {
      shows:
        'Para cada estado de conexión, el mayor tiempo que cualquier sesión de cliente llevaba en él desde su último cambio de estado, en minutos.',
      look: 'Un tiempo largo en active es una consulta lenta. Un tiempo largo en idle in transaction mantiene bloqueos y frena el vacuum, e idle_in_transaction_session_timeout puede terminar esas sesiones. Los tiempos largos en idle no hacen daño por sí solos.',
    },
    'wait-events': {
      shows:
        'Backends activos en cada muestra, por el tipo de evento de espera que informaron. Sin evento de espera significa que PostgreSQL no informó ninguno. El backend estaba en ejecución en una CPU o esperando a que el sistema operativo lo planificara.',
      look: 'Las esperas Lock indican que las sesiones se bloquearon entre sí. Las esperas IO son tiempo esperando lecturas, escrituras o flushes, así que compárelas con los gráficos de latencia de disco. Las esperas LWLock apuntan a contención dentro de PostgreSQL, por ejemplo en el WAL o en el mapeo de buffers. Muchos backends sin evento de espera en un host ocupado pueden indicar que hacen cola por la CPU.',
    },
    'deadlocks-and-conflicts': {
      shows: 'Deadlocks, conflictos de recuperación y fallos de checksum por intervalo, en todas las bases de datos.',
      look: 'Cada deadlock aborta una transacción. La solución está en la aplicación, normalmente tomando los bloqueos en el mismo orden en todas partes. Los conflictos de recuperación ocurren en los standbys. Cualquier fallo de checksum indica que hay datos corruptos en disco. Revise el log del servidor y el almacenamiento el mismo día.',
    },
    'sessions-ended-abnormally': {
      shows:
        'Sesiones por intervalo que terminaron sin una desconexión limpia. Abandonadas significa que el cliente desapareció. Fatal significa que un error terminó la sesión, incluidos timeouts como idle_in_transaction_session_timeout. Terminadas significa que un operador la terminó, por ejemplo con pg_terminate_backend.',
      look: 'Las sesiones abandonadas apuntan a problemas de red, caídas del cliente o un pooler que corta conexiones. Las sesiones Fatal vienen de errores y timeouts, y el log del servidor tiene la causa de cada una. Las sesiones terminadas corresponden a llamadas a pg_terminate_backend de un operador o de una herramienta.',
    },
    'block-i-o-time': {
      shows:
        'Tiempo que los backends dedicaron a leer y escribir bloques de datos, en milisegundos por segundo. Necesita track_io_timing activado.',
      look: 'Un valor de 1000 significa que un backend, de media, pasó todo su tiempo en I/O. Un tiempo de lectura que sube mientras baja la Tasa de aciertos de caché indica que el working set no cabe en memoria. Compare con la latencia de disco para distinguir un disco lento de un exceso de lecturas.',
    },
    'standby-conflicts': {
      shows:
        'Consultas en un standby que la recuperación canceló, por intervalo y por causa. Un conflicto de snapshot ocurre cuando el vacuum en el primario eliminó filas que una consulta en el standby aún necesitaba.',
      look: 'Cualquier valor por encima de cero indica que los usuarios del standby vieron errores. Los conflictos de snapshot son los más comunes. hot_standby_feedback o un max_standby_streaming_delay más largo los reducen, a costa de más bloat en el primario o más retraso en el standby.',
    },
    'wal-archiving': {
      shows: 'Ficheros WAL archivados, y fallos de archivado, por intervalo, de pg_stat_archiver.',
      look: 'Cualquier fallo indica que el archivado se retrasa, y la recuperación point-in-time (PITR) no puede ir más allá del último fichero archivado. PostgreSQL conserva cada fichero WAL que no pudo archivar, así que pg_wal crece hasta que el comando de archivado vuelve a funcionar y puede llenar el disco. Los ficheros archivados deberían seguir el ritmo del WAL generado.',
    },
    'replication-slots': {
      shows:
        'WAL que PostgreSQL conserva en disco para los slots de replicación, como la mayor cantidad retenida por cualquier slot de cada tipo, físico o lógico.',
      look: 'Un slot conserva el WAL hasta que su consumidor lo confirma. Una línea que solo crece indica que un consumidor es lento, está caído o ya no existe. Sin max_slot_wal_keep_size, ese WAL puede llenar el disco y detener el servidor. Elimine los slots que nadie usa.',
    },
    'slru-reads': {
      shows:
        'Bloques leídos de disco por segundo por las cachés SLRU. Estas cachés pequeñas guardan el estado de las transacciones, las subtransacciones, los multixacts y las marcas de tiempo de commit.',
      look: 'Los valores cerca de cero son normales. Las lecturas constantes de subtransacciones vienen de un uso intensivo de savepoints o de bloques de excepción en PL/pgSQL, y pueden ralentizar todo el servidor. Las lecturas de multixact vienen de muchas sesiones que bloquean las mismas filas, por ejemplo con SELECT FOR SHARE o con claves foráneas.',
    },
    'i-o-by-operation': {
      shows:
        'Datos leídos, escritos y extendidos por segundo por todos los backends, de pg_stat_io. Los extends son bloques nuevos añadidos al final de un fichero de tabla o de índice.',
      look: 'Las lecturas grandes apuntan a escaneos de tablas que no caben en memoria. Las escrituras deberían seguir a los checkpoints y al background writer, así que compare con Escrituras de WAL y checkpoint. Los extends constantes son normales en tablas que crecen.',
    },
    rows: {
      shows:
        'Filas por segundo en todas las bases de datos. Lectura son las filas devueltas por los escaneos. Escritura son las filas insertadas, actualizadas y eliminadas.',
      look: 'Lectura cuenta cada fila que devolvió un escaneo, antes de los filtros, así que una tasa de lectura alta con pocas transacciones apunta a escaneos secuenciales de tablas grandes. Una tasa de escritura que sube con el WAL generado es normal. Compare la forma con Transacciones por segundo.',
    },
  },
}

export default translation
