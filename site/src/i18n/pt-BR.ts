import type { Dictionary } from './en';

const ptBR: Dictionary = {
  meta: {
    title: 'pg-probe: diagnóstico de PostgreSQL em um arquivo',
    description:
      'Seu cliente roda um comando no servidor PostgreSQL e manda um arquivo para você. Analise em um console local e imprima um relatório em português, inglês ou espanhol.',
  },
  language: {
    label: 'Idioma',
    browser: 'Idioma do navegador',
  },
  nav: {
    how: 'Como funciona',
    report: 'Relatório',
    top: 'Ao vivo',
  },
  hero: {
    eyebrow: 'Para quem trabalha com PostgreSQL',
    title: 'Seu cliente roda um comando. Você vê o quadro completo.',
    lead: 'O pg-probe grava as métricas do PostgreSQL e do host Linux em um único arquivo. Abra em um console local, ou imprima um relatório que seu cliente consegue ler.',
    install: 'Instalar o pg-probe',
    howEyebrow: 'Como funciona',
    howTitle: 'Captura lá. Análise aqui.',
  },
  flow: {
    label: 'Como o pg-probe é usado',
    capture: 'Captura',
    captureWhere: 'No servidor do cliente',
    handoff: 'O arquivo muda de mãos',
    handoffWhere: 'fora do pg-probe',
    review: 'Análise',
    reviewWhere: 'No seu notebook',
    captureNote: 'O resultado é um arquivo no diretório atual. Nada sai do servidor.',
    terminal: 'Terminal, no seu notebook',
    consoleTitle: 'O console de análise do pg-probe',
    reviewNote: 'Abra o endereço no navegador. O console lê o mesmo arquivo, na sua máquina.',
  },
  handoff: {
    from: 'De',
    to: 'Para',
    you: 'você',
    subject: 'Assunto',
    subjectText: 'Captura do db-prod-01',
    body: 'Segue a captura da lentidão de hoje de manhã.',
    note: 'Não é uma etapa do pg-probe. Seu cliente manda o arquivo como manda qualquer arquivo: e-mail, chamado ou pasta compartilhada. O pg-probe não envia nada.',
  },
  report: {
    eyebrow: 'O relatório impresso',
    title: 'Um relatório que seu cliente consegue ler.',
    lead: 'Imprima a sessão inteira em PDF pelo console: 39 gráficos do host e do banco de dados, de CPU e discos a locks e WAL. Cada gráfico ganha uma página, com o que ele mede e como ler.',
    ticks: [
      '<b>Capa e sumário</b>, com link para cada gráfico',
      '<b>"O que observar"</b> em cada página, escrito para quem não conhece a métrica',
      '<b>Fontes que não estavam disponíveis</b> listadas depois da capa, nunca representadas como zero',
    ],
    languages: 'Em português, inglês ou espanhol, na língua que seu cliente lê.',
    coverAlt: 'Capa do relatório',
    pageAlt: 'Uma página do relatório: pedidos de lock em espera, com o quadro O que observar acima do gráfico',
  },
  console: {
    eyebrow: 'O console',
    title: 'Feito para ler uma sessão.',
    lead: 'Clique em um gráfico para fixar um momento, ou arraste para selecionar um intervalo. Todos os gráficos e legendas acompanham.',
    fine: 'Na captura de demonstração, no instante fixado às 15:04:34 UTC, a taxa de transações caiu para 2,6 por segundo, com 32 pedidos de lock em espera e 3,4% de uso da CPU.',
    features: [
      { title: 'Percentis, não médias', text: 'Toda legenda mostra p50, p75, p95, p99 e o valor atual.' },
      {
        title: 'Taxas a partir das amostras',
        text: '<code>--duration</code> e <code>--interval</code> registram como os contadores mudam. Um reset de contador é uma lacuna, nunca um valor.',
      },
      {
        title: 'Seu próprio SQL',
        text: 'O artefato é um banco de dados SQLite. Abra com o <code>sqlite3</code> quando o console não bastar.',
      },
    ],
  },
  security: {
    eyebrow: 'Para a equipe de segurança do seu cliente',
    title: 'Sem agente. Sem conta. Sem upload.',
    lead: 'Seu cliente não instala uma stack de monitoramento, e você não precisa de acesso ao servidor dele.',
    features: [
      {
        title: 'Sessão somente leitura',
        text: 'Uma conexão com <code>default_transaction_read_only=on</code> e um statement timeout.',
      },
      {
        title: 'Lista de permissões estrita',
        text: 'Nada de credenciais, variáveis de ambiente, linhas de comando ou identificadores de rede no arquivo.',
      },
      {
        title: 'Inspecionável',
        text: 'Seu cliente pode abrir o arquivo antes de enviar. Nada sai do servidor sozinho.',
      },
    ],
  },
  top: {
    eyebrow: 'Também no binário',
    title: '<code class="cmd-title">pg-probe top</code>, para chamadas ao vivo.',
    lead: 'Quando você está em uma chamada com o cliente, veja as consultas que estão rodando agora, das mais longas para as mais curtas. Funciona a partir do PostgreSQL 9.3.',
    views: 'Telas do pg-probe top',
    running: 'Em execução',
    runningNote: 'Todos os backends ativos, do maior para o menor tempo de execução, com sua CPU, I/O e espera de I/O, lidos do <code>/proc</code>.',
    waiting: 'Em espera',
    waitingNote: 'Backends esperando por um lock, e o lock que cada um espera.',
    blocking: 'Bloqueando',
    blockingNote: 'Quem segura o lock, com a fila atrás de cada um em forma de árvore.',
    vacuum: 'Vacuum',
    vacuumNote: 'Todo autovacuum e vacuum manual, com sua fase e quanto da tabela já foi lido.',
    query: 'Consulta',
    queryNote: 'Um backend: a consulta completa, todos os tempos decorridos e sua CPU, memória e I/O.',
    plan: 'Plano',
    planNote: 'O plano estimado. <code>EXPLAIN</code> sem <code>ANALYZE</code>, então a consulta não roda de novo.',
    function: 'Função',
    functionNote: 'A função plpgsql que ela chama: assinatura, estatísticas de chamadas e código-fonte.',
    cost: 'Quanto custa',
    connection: 'conexão somente leitura',
    refresh: 'por atualização',
    core: 'de um núcleo',
    costFine:
      'Medido em um servidor de teste com 206 conexões sob pgbench. Um SELECT em pg_stat_activity por atualização. As telas de lock, a página da consulta e os popups adicionam uma consulta cada, só enquanto estão abertos.',
  },
  install: {
    eyebrow: 'Instalação',
    title: 'Roda nos servidores que seus clientes têm.',
    postgres: '9.3 e posteriores',
    postgresText: 'Versões do PostgreSQL',
    el: 'EL7 e posteriores',
    elText: 'RHEL, CentOS, Rocky. rpm, testado no Rocky Linux 9',
    ubuntu: 'Ubuntu 18.04 e posteriores',
    ubuntuText: 'deb, testado no Ubuntu 24.04',
    archText: 'binário estático, sem dependências',
    copy: 'Copiar o comando de instalação',
    download: 'Baixar para Linux',
    downloadText: 'deb, rpm ou um binário estático, sem dependências.',
    fine: 'Código aberto sob a PostgreSQL License. A captura, o console e o relatório vêm no mesmo binário.',
  },
  footer: {
    license: 'PostgreSQL License',
    releases: 'Versões',
  },
};

export default ptBR;
