import type { Translate } from './ui';

const navigation: Record<string, string> = {
  Library: 'roonLibrary', Search: 'search', Artists: 'artists', Albums: 'albums', Tracks: 'tracks',
  Composers: 'composers', Tags: 'tags', Playlists: 'playlists', Genres: 'genres', Folders: 'folders',
  'Recently Added': 'recentlyAdded', 'Search results': 'searchResults'
};

const prompts: Record<string, string> = {
  ...navigation, Go: 'confirm', Continue: 'continue', 'Search Roon': 'searchRoon',
  'Artist, album or track': 'searchMusicPrompt', 'Search Artists': 'searchArtists',
  'Search Albums': 'searchAlbums', 'Search Tracks': 'searchTracks'
};

const actions: Record<string, string> = {
  'play now': 'playNow', 'add next': 'addNext', queue: 'addQueue', 'add to queue': 'addQueue',
  'start radio': 'startRadio', shuffle: 'shuffle', play: 'play', pause: 'pause', stop: 'stop'
};

export interface BrowseView {
  titleKey?: string;
  navigation: boolean;
}

export function navigationKey(value?: string) {
  return value ? navigation[value] : undefined;
}

export function browseLabel(value: string | undefined, kind: 'navigation' | 'prompt' | 'action', text: Translate) {
  if (!value) return '';
  const key = kind === 'action' ? actions[value.toLowerCase()] : (kind === 'prompt' ? prompts : navigation)[value];
  return key ? text(key) : value;
}
