import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';

let server, Dialog;
const firstId='11111111-2222-4333-8444-555555555555';
const secondId='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
const workspace={id:'fixture',name:'Fixture',root_path:'/fixture',created_at:'2026-01-01T00:00:00Z'};
const profile=id=>({id,name:'Imported Claude',provider:'claude_code',endpoint_url:null,model:null,permission_mode:'native',secret_ref:null,native_config:{source_id:'claude-default',config_dir:'/fixture/claude',config_env:null},created_at:workspace.created_at});

before(async()=>{
  // Compile the actual SFC and its template. No listening HTTP/HMR server,
  // browser context, user credentials, network fetches or native client starts.
  server=await createServer({configFile:false,root:fileURLToPath(new URL('../../',import.meta.url)),plugins:[vue()],server:{middlewareMode:true,hmr:false,ws:false},appType:'custom',optimizeDeps:{noDiscovery:true}});
  Dialog=(await server.ssrLoadModule('/src/features/CreateSessionDialog.vue')).default;
});
after(async()=>{await server?.close();});
async function render(profiles,initialProvider='claude_code') {
  const context={};
  const output=await renderToString(createSSRApp(Dialog,{workspace,profiles,initialProvider}),context);
  return output+Object.values(context.teleports??{}).join('');
}
function selectedOptions(html) {return html.match(/<option\b[^>]*>/g)?.filter(tag=>/\bselected\b/.test(tag))??[];}

test('new-session SFC actually selects the only imported Claude profile, not isolation',async()=>{
  const html=await render([profile(firstId)]);
  assert.ok(selectedOptions(html).some(tag=>tag.includes(firstId)));
  assert.ok(!selectedOptions(html).some(tag=>tag.includes('value=""')));
  assert.ok(!html.includes('session-model-options'));
  assert.ok(html.includes('/fixture/claude'));
});
test('multiple imported accounts keep creation disabled until an explicit selection',async()=>{
  const html=await render([profile(firstId),profile(secondId)]);
  assert.ok(selectedOptions(html).some(tag=>tag.includes('__choose_profile__')));
  const create=html.match(/<button\b[^>]*class="primary-button"[^>]*>/g)?.at(-1);
  assert.ok(create&&/\bdisabled\b/.test(create));
  assert.ok(!html.includes('session-model-options'));
});
test('a Claude import does not become the default profile for Codex; with none, Codex asks for one',async()=>{
  const html=await render([profile(firstId)],'codex');
  assert.ok(!html.includes(firstId));
  // No blank home with no sign-in is offered in its place.
  assert.ok(!html.includes('<option value=""'));
  assert.ok(html.includes('session-no-profile'));
  const create=html.match(/<button\b[^>]*class="primary-button"[^>]*>/g)?.at(-1);
  assert.ok(create&&/\bdisabled\b/.test(create));
});
