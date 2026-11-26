/** Whisper languages people actually pick, by endonym-free English name. */
export const LANGUAGES: { code: string; name: string }[] = [
  ['en', 'English'], ['es', 'Spanish'], ['fr', 'French'], ['de', 'German'], ['it', 'Italian'], ['pt', 'Portuguese'],
  ['nl', 'Dutch'], ['el', 'Greek'], ['pl', 'Polish'], ['ru', 'Russian'], ['uk', 'Ukrainian'], ['cs', 'Czech'],
  ['sv', 'Swedish'], ['da', 'Danish'], ['no', 'Norwegian'], ['fi', 'Finnish'], ['ro', 'Romanian'], ['hu', 'Hungarian'],
  ['bg', 'Bulgarian'], ['hr', 'Croatian'], ['sr', 'Serbian'], ['sk', 'Slovak'], ['sl', 'Slovenian'], ['tr', 'Turkish'],
  ['ar', 'Arabic'], ['he', 'Hebrew'], ['fa', 'Persian'], ['hi', 'Hindi'], ['ur', 'Urdu'], ['bn', 'Bengali'],
  ['ta', 'Tamil'], ['zh', 'Chinese'], ['ja', 'Japanese'], ['ko', 'Korean'], ['vi', 'Vietnamese'], ['th', 'Thai'],
  ['id', 'Indonesian'], ['ms', 'Malay'], ['tl', 'Tagalog'], ['ca', 'Catalan'], ['lt', 'Lithuanian'], ['lv', 'Latvian'],
  ['et', 'Estonian'], ['sw', 'Swahili'], ['af', 'Afrikaans'],
].map(([code, name]) => ({ code, name }));

export const languageName = (code: string) => LANGUAGES.find((l) => l.code === code)?.name ?? code.toUpperCase();
