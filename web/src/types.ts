export interface Core {
  core_id: string;
  name?: string;
  host: string;
  connection: string;
  error?: string | null;
}

export interface Volume {
  type: string;
  min: number;
  max: number;
  step?: number;
  value: number;
  is_muted?: boolean;
}

export interface Output {
  output_id: string;
  display_name: string;
  volume?: Volume;
}

export interface Track {
  length?: number;
  image_key?: string;
  seek_position?: number;
  one_line?: { line1: string; line2?: string; line3?: string };
  two_line?: { line1: string; line2?: string; line3?: string };
  three_line?: { line1: string; line2?: string; line3?: string };
}

export interface Zone {
  zone_id: string;
  display_name: string;
  state: string;
  is_previous_allowed?: boolean;
  is_next_allowed?: boolean;
  is_pause_allowed?: boolean;
  is_play_allowed?: boolean;
  is_seek_allowed?: boolean;
  seek_position?: number;
  now_playing?: Track | null;
  outputs?: Output[];
  [field: string]: unknown;
}

export interface Player {
  cores: Core[];
  selected_core_id: string | null;
  zones: Zone[];
  discovery_error?: string | null;
}

export interface BrowseItem {
  title: string;
  subtitle?: string;
  item_key?: string;
  image_key?: string;
  hint?: string;
  input_prompt?: { prompt?: string; action?: string; value?: string; is_password?: boolean };
}

export interface BrowseResult {
  action?: string;
  message?: string;
  is_error?: boolean;
  list?: { title: string; count: number; level: number; display_offset?: number } | null;
  item?: BrowseItem;
  items: BrowseItem[];
  offset: number;
}

export interface BrowseOptions {
  item_key?: string;
  pop_all?: boolean;
  pop_levels?: number;
  input?: string;
  offset?: number;
}

export interface RuntimeConfig {
  name: string;
  broadcast_interval_secs: number;
  live_proxy_timeout_ms: number;
  reverse_proxy: boolean;
  proxy_ip: string | null;
}

export interface Status {
  local_ip?: string;
  remote_ip?: string | null;
  forwarding_enabled?: boolean;
  use_broadcast?: boolean;
  sood_port?: number;
  discovery_port?: number;
  uptime_secs?: number;
  version?: string;
  config?: RuntimeConfig;
  metrics?: { sood_queries?: number; live_core_responses?: number; template_fallbacks?: number; tcp_connections?: number };
}

export interface AudioStatus {
  enabled: boolean;
  available: boolean;
  state: string;
  name?: string;
  listeners?: number;
  error?: string | null;
  pcm_bytes_received?: number;
  bytes_streamed?: number;
  receiver_diagnostics?: string | null;
  encoder_diagnostics?: string | null;
}

export interface AudioNetwork {
  interface: string | null;
  interfaces: { name: string; addresses: string[] }[];
}

export interface Auth {
  required: boolean;
  authenticated: boolean;
}
