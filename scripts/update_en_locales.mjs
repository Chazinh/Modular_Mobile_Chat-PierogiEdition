import fs from 'fs';

const enPath = 'client/src/locales/en.json';
const en = JSON.parse(fs.readFileSync(enPath, 'utf8'));

// High quality, natural English replacements
const updates = {
  // Chat vs Conversation
  "Conversas": "Chats",
  "Buscar conversas e mensagens": "Search chats and messages",
  "Nenhuma conversa ainda.": "No chats yet.",
  "Renomear conversa": "Rename chat",
  "Duplicar personagem (conversa limpa)": "Duplicate character (fresh chat)",
  "Conversa duplicada": "Chat duplicated",
  "Exportar conversa": "Export chat",
  "Conversa renomeada": "Chat renamed",
  "Comece a conversa. Personalize o personagem em": "Start the chat. Customize the character in",
  "Crie perfis para quem você interpreta e troque entre eles em cada conversa (⋯ → Persona).": "Create personas and switch between them in each chat (⋯ → Persona).",
  "{0} conversa(s) importada(s){1}.": "{0} chat(s) imported{1}.",
  "Apagar o histórico de TODAS as conversas? Vai para a lixeira por 30 dias e é recusado se houver resposta em andamento.": "Delete history for ALL chats? It will be kept in trash for 30 days and cannot be done while generating.",
  "Estado sincronizado com a conversa.": "State synchronized with the chat.",
  "· com fonte na conversa": "· sourced from chat",
  "Persona desta conversa": "Persona for this chat",
  "Toque em um perfil para usá-lo nesta conversa; vale para as próximas respostas.": "Tap a profile to use it in this chat for upcoming responses.",
  "Apagar o perfil \"{0}\"? Conversas que o usam mantêm o nome e a descrição atuais.": "Delete profile \"{0}\"? Chats using it will keep their current name and description.",
  "Nome da conversa": "Chat name",
  "O PC guarda as conversas e a memória.": "The PC stores all chats and memory.",
  "Apagar TODO o histórico desta conversa? Fica na lixeira por 30 dias e a memória derivada é invalidada.": "Delete ALL history for this chat? Kept in trash for 30 days and derived memories will be cleared.",
  "· nesta conversa": "· in this chat",
  "A conversa inteira fica guardada no PC. A cada resposta o modelo recebe: regras, personalização, persona, cena atual, quem sabe o quê, threads, até 8 memórias relevantes e os últimos": "The entire chat is stored on your PC. For each response the model receives: rules, character card, persona, current scene, context threads, up to 8 relevant memories, and the latest",
  "Apagar histórico desta conversa": "Delete chat history",
  "Duplicar personagem em conversa limpa": "Duplicate character in a fresh chat",
  "Esforço desta conversa": "Reasoning effort for this chat",
  "Uma reação curta por personagem relevante e uma pausa para você responder, sem longas conversas entre eles.": "A short reaction per relevant character and a pause for you to respond, without long dialogues between them.",
  "A escolha fica salva neste aparelho ou navegador. Não muda o idioma das conversas nem das respostas do modelo.": "Saved locally on this device or browser. Does not change chat content or AI language.",
  "Conversa “{0}”{1}": "Chat “{0}”{1}",
  "A conversa inteira fica guardada no servidor de origem (PC ou aparelho). A cada resposta o modelo recebe: regras, personalização, persona, cena atual, quem sabe o quê, threads, até 8 memórias relevantes e os últimos": "The full chat is stored on the origin server (PC or device). For each reply, the model receives: rules, character card, persona, current scene, context threads, up to 8 relevant memories, and the last",
  "Ligado: inclui as regras do app na primeira mensagem enviada ao modelo, antes da conversa. Desligado: o app escolhe o formato compatível com a rota. Suas mensagens salvas não são alteradas.": "On: includes app rules in the first message sent to the model, before the chat. Off: the app chooses the format supported by the route. Your saved messages are unchanged.",

  // New additions & confirmations
  "Deseja criar uma nova conversa?": "Start a new chat?",
  " Novo Chat": " New Chat",
  "Novo Chat": "New Chat",
  "Novo chat": "New chat",
  " Novo Chat\n              ": " New Chat",

  // Character card & Cast
  "O elenco vem da sua ficha (.txt) e da narrativa. Confira os nomes: eles definem": "The cast comes from your character card (.txt) and the narrative. Check the names: they determine",
  "Ficha relida: {0} personagem(ns){1}.": "Character card reloaded: {0} character(s){1}.",
  "Reanalisar ficha": "Re-analyze character card",
  "Nenhum personagem ainda. A ficha será analisada ao salvar.": "No characters yet. The character card will be analyzed when saved.",
  "da ficha .txt": "from .txt character card",
  "Personalização (.txt)": "Character card (.txt)",
  "Personalização": "Character card",
  "⋯ → Personalização": "⋯ → Character card",
  "Cada perfil é quem você interpreta: nome, descrição (enviada ao modelo como sua ficha) e avatar.": "Each profile is your persona: name, description (sent to the model as your character card), and avatar.",
  "Descrição (idade, aparência, origem, jeito...) — vai para o modelo como sua ficha": "Description (age, appearance, background, personality...) — sent to the model as your persona card",
  "Características, regras de mundo e personalidade. A ficha fica criptografada no PC. Para editá-la por fora, importe um arquivo .txt; cada alteração cria uma nova revisão.": "Traits, world lore, and personality. The character card is encrypted on your PC. To edit externally, import a .txt file; each edit creates a new revision.",
  "Características, regras de mundo e personalidade. A ficha fica criptografada no servidor de origem (PC ou aparelho). Para editá-la por fora, importe um arquivo .txt; cada alteração cria uma nova revisão.": "Traits, world lore, and personality. The character card is encrypted on the origin server (PC or device). To edit externally, import a .txt file; each edit creates a new revision.",

  // Controls & Settings polish
  "Modelo e esforço": "Model & effort",
  "Pensamento do modelo": "Reasoning",
  "Continuar a cena": "Continue scene",
  "Refazer a última resposta": "Regenerate response",
  "Pares recentes preferidos": "Recent turn pairs",
  "Estilo e limites (global)": "Style & limits (global)",
  "Carregando presets do PC…": "Loading presets from PC…",
  "Carregando presets do servidor ativo…": "Loading presets from active server…"
};

let count = 0;
for (const [k, v] of Object.entries(updates)) {
  en[k] = v;
  count++;
}

fs.writeFileSync(enPath, JSON.stringify(en, null, 2) + '\n', 'utf8');
console.log(`Updated ${count} translation keys in ${enPath}`);
