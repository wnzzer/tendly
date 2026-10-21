<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, reactive, ref, watch } from 'vue';
import type { CompletionSource } from '@codemirror/autocomplete';
import { request, json, errorMessage } from './api';
import { BALANCE_EXAMPLE, BALANCE_EXAMPLES, authFields, authFromFields, type BalanceConfig, type BalanceReading, type BalanceView } from './balance-model';
import { applyView } from './balance-store';
import BalanceResult from './BalanceResult.vue';
import Icon from './Icon.vue';
import { useI18n } from '../i18n';
// CodeMirror loads with the first balance editor, not with the app.
const CodeEditor=defineAsyncComponent(()=>import('./CodeEditor.vue'));
const props=defineProps<{profileId:string; disabled?:boolean}>();
const {t}=useI18n();
const mode=ref<'disabled'|'money'|'percentage'>('disabled'), currency=ref('USD'), queryUrl=ref(''), secret=ref(''), script=ref(BALANCE_EXAMPLE);
const loaded=ref(false), busy=ref(false), error=ref(''), view=ref<BalanceView>(), preview=ref<BalanceReading>();
const baseline=ref('');
const auth=reactive(authFields({kind:'header',name:'Authorization',value:'Bearer {key}'}));
const url=`/endpoint-profiles/${encodeURIComponent(props.profileId)}/balance`;
const MODES=[{id:'disabled',label:'Disabled'},{id:'money',label:'Fiat balance'},{id:'percentage',label:'Remaining percentage'}] as const;
function config():BalanceConfig|null {
  if(mode.value==='disabled')return null;
  return {unit:mode.value==='money'?{kind:'money',currency:currency.value.trim().toUpperCase()}:{kind:'percentage'},query_url:queryUrl.value.trim(),auth:authFromFields(auth),secret_ref:secret.value.trim()||null,script:script.value};
}
watch(() => JSON.stringify(config()), () => { preview.value=undefined; }, {flush:'sync'});
const dirty=computed(()=>loaded.value&&JSON.stringify(config())!==baseline.value);
const disabled=computed(()=>busy.value||props.disabled||!loaded.value);
function apply(next:BalanceView){
  view.value=next;const c=next.config;
  mode.value=c?.unit.kind??'disabled';currency.value=c?.unit.kind==='money'?c.unit.currency:'USD';queryUrl.value=c?.query_url??'';Object.assign(auth,authFields(c?.auth??{kind:'header',name:'Authorization',value:'Bearer {key}'}));secret.value=c?.secret_ref??'';script.value=c?.script??BALANCE_EXAMPLE;
  baseline.value=JSON.stringify(config());loaded.value=true;preview.value=undefined;
}
async function load(){error.value='';try{apply(await request<BalanceView>(url));}catch(cause){error.value=errorMessage(cause);}}
onMounted(load);
function reset(){if(view.value)apply(view.value);}
/**
 * Saved as it is edited, like the profile around it: once what a query needs
 * is filled in, and for text, when the field is left. A half-written query is
 * not sent to be refused; the header says what it still needs.
 */
const ready=computed(()=>mode.value==='disabled'||(!!queryUrl.value.trim()&&!!script.value.trim()&&(mode.value!=='money'||/^[A-Za-z]{3}$/.test(currency.value.trim()))));
const saved=ref(false);
let timer:ReturnType<typeof setTimeout>|undefined, saving:Promise<void>|undefined;
const root=ref<HTMLElement>();
function typing(){const active=document.activeElement as HTMLElement|null;return !!active&&!!root.value?.contains(active)&&(active.isContentEditable||active instanceof HTMLTextAreaElement||(active instanceof HTMLInputElement&&active.type!=='checkbox'));}
watch(()=>dirty.value&&JSON.stringify(config()),value=>{saved.value=false;if(!value)return;clearTimeout(timer);timer=setTimeout(()=>{if(!typing())void flush();},300);});
async function flush():Promise<void>{
  clearTimeout(timer);
  if(saving){await saving;}
  if(!dirty.value||!ready.value||props.disabled)return;
  saving=save().finally(()=>{saving=undefined;});
  return saving;
}
async function save(){
  const sent=JSON.stringify(config());
  busy.value=true;error.value='';
  try{const next=await request<BalanceView>(url,json('PUT',config()));view.value=next;applyView(props.profileId,next);baseline.value=sent;saved.value=JSON.stringify(config())===sent;}
  catch(cause){error.value=errorMessage(cause);baseline.value=sent;}
  finally{busy.value=false;}
}
/** Leaving a field saves it; the next field taking focus is not typing yet. */
function leaveField(){setTimeout(()=>void flush());}
defineExpose({dirty,busy,ready,reset,flush});
async function test(){busy.value=true;error.value='';preview.value=undefined;try{preview.value=await request<BalanceReading>(url+'/test',json('POST',config()));}catch(cause){error.value=errorMessage(cause);}finally{busy.value=false;}}
async function refresh(){busy.value=true;error.value='';try{view.value=await request<BalanceView>(url+'/query',json('POST'));applyView(props.profileId,view.value);preview.value=undefined;}catch(cause){error.value=errorMessage(cause);}finally{busy.value=false;}}
/** The help sentence with its code set apart; the code reads the same in every language. */
const codeParts=(text:string)=>text.split(/(await ctx\.request\(\)|\{ value \})/);
function useExample(event:Event){const select=event.target as HTMLSelectElement;const example=BALANCE_EXAMPLES.find(entry=>entry.id===select.value);if(example)script.value=example.script;select.value='';}

/** What a script can reach: ctx.request(), and the words its body is built from. */
const completions:CompletionSource=context=>{
  const member=context.matchBefore(/ctx\.\w*/);
  if(member)return {from:member.from+4,options:[{label:'request',type:'method',detail:'(options?) → JSON',apply:'request()',info:t('GET the query URL, or pass { url, method, headers, body }. Returns parsed JSON.')}]};
  const word=context.matchBefore(/\w+/);
  if(!word||(word.from===word.to&&!context.explicit))return null;
  return {from:word.from,options:[
    {label:'ctx',type:'variable',detail:'Tendly'},
    {label:'await ctx.request()',type:'function',detail:'→ JSON'},
    {label:'return { value }',type:'keyword',apply:'return { value:  };',detail:t('the balance')},
  ]};
};
</script>
<template>
  <section ref="root" class="setting-card balance-editor" :aria-label="t('Custom balance query')" @focusout="leaveField">
    <header class="setting-card-head balance-editor-head">
      <span class="setting-mark"><Icon name="wallet" :size="16" /></span><strong>{{ t('Custom balance query') }}</strong>
      <span v-if="busy&&dirty" class="balance-state">{{ t('Saving…') }}</span>
      <span v-else-if="dirty&&!ready" class="balance-state warn">{{ t(mode==='money'&&queryUrl.trim()?'Saves once the currency is a 3-letter code':'Saves once the query URL and script are filled in') }}</span>
      <span v-else-if="saved" class="balance-state ok">{{ t('Saved') }}</span>
      <div class="balance-modes setting-card-aside" role="radiogroup" :aria-label="t('Balance type')">
        <button v-for="entry in MODES" :key="entry.id" type="button" role="radio" :aria-checked="mode===entry.id" :class="{selected:mode===entry.id}" @click="mode=entry.id">{{ t(entry.label) }}</button>
      </div>
    </header>
    <p v-if="error" class="inline-error" role="alert">{{ t(error) }} <button v-if="!loaded" type="button" class="text-button" @click="load">{{ t('Retry') }}</button></p>
    <fieldset class="balance-fields" :disabled="disabled">
      <template v-if="mode!=='disabled'">
        <div class="balance-rows">
          <label class="setting-row"><span class="setting-label"><strong>{{ t('Balance query URL') }}</strong><small>{{ t('Where the script reads the balance from.') }}</small></span><input v-model="queryUrl" type="url" class="field-mono" placeholder="https://api.example.com/balance" autocomplete="off" spellcheck="false" /></label>
          <label v-if="mode==='money'" class="setting-row"><span class="setting-label"><strong>{{ t('Currency code') }}</strong><small>{{ t('Three letters, such as USD or CNY.') }}</small></span><input v-model="currency" maxlength="3" placeholder="USD / CNY" spellcheck="false" autocomplete="off" class="field-mono balance-currency" /></label>
          <label class="setting-row"><span class="setting-label"><strong>{{ t('Authentication') }}</strong><small>{{ t('How the query sends the key.') }}</small></span><select v-model="auth.preset"><option value="bearer">{{ t('Bearer token (Authorization header)') }}</option><option value="api_key">{{ t('x-api-key header') }}</option><option value="header">{{ t('Custom header') }}</option><option value="query">{{ t('Query parameter') }}</option><option value="none">{{ t('No authentication') }}</option></select></label>
          <div v-if="auth.preset==='header'" class="setting-row"><span class="setting-label"><strong>{{ t('Custom header') }}</strong><small>{{ t('{key} is replaced by the key on the server.') }}</small></span><span class="setting-pair"><input :aria-label="t('Header name')" v-model="auth.headerName" placeholder="X-Token" autocomplete="off" spellcheck="false" class="field-mono" /><input :aria-label="t('Header value')" v-model="auth.headerValue" placeholder="Bearer {key}" autocomplete="off" spellcheck="false" class="field-mono" /></span></div>
          <label v-else-if="auth.preset==='query'" class="setting-row"><span class="setting-label"><strong>{{ t('Parameter name') }}</strong></span><input v-model="auth.queryName" placeholder="api_key" autocomplete="off" spellcheck="false" class="field-mono" /></label>
          <label v-if="auth.preset!=='none'" class="setting-row"><span class="setting-label"><strong>{{ t('Balance key reference') }}</strong><small>{{ t('Empty uses the endpoint key.') }}</small></span><input v-model="secret" class="field-mono" :placeholder="t('Empty uses the endpoint key · env:TENDLY_SECRET_…')" autocomplete="off" spellcheck="false" /></label>
        </div>
        <div class="setting-row stack balance-script">
          <div class="balance-script-head">
            <span>{{ t('Balance JavaScript') }}</span>
            <select class="balance-examples" :aria-label="t('Insert an example')" @change="useExample"><option value="">{{ t('Insert an example…') }}</option><option v-for="entry in BALANCE_EXAMPLES" :key="entry.id" :value="entry.id">{{ t(entry.label) }}</option></select>
          </div>
          <CodeEditor v-model="script" :label="t('Balance JavaScript')" :disabled="disabled" :completions="completions" />
          <p class="form-help"><template v-for="(part,index) in codeParts(t(mode==='money'?'An async function body: read JSON with await ctx.request(), then return { value }. A decimal string keeps money exact.':'An async function body: read JSON with await ctx.request(), then return { value } as the percentage left, 0–100.'))" :key="index"><code v-if="index%2">{{ part }}</code><template v-else>{{ part }}</template></template></p>
          <details class="balance-api-help"><summary>{{ t('Request options') }}</summary><pre><code>await ctx.request({
  url: '/balance',            // {{ t('same origin as the query URL') }}
  method: 'GET',              // GET | POST
  headers: { Accept: 'application/json' },
  body: { account: 'me' },    // {{ t('sent as JSON') }}
})</code></pre><p class="form-help">{{ t('At most 4 requests, 10 seconds and 1 MiB per response.') }}</p></details>
        </div>
      </template>
    </fieldset>
    <footer class="setting-row balance-editor-foot">
      <div class="balance-outcome">
        <div v-if="preview" class="balance-preview"><strong>{{ t('Test result') }}</strong><BalanceResult :reading="preview" /></div>
        <BalanceResult v-else-if="view?.config&&!dirty" :reading="view.reading" :stale="view.stale" :error="view.error" />
      </div>
      <div class="toolbar-buttons">
        <button v-if="view?.config" type="button" class="small-button" :disabled="disabled||dirty" @click="refresh"><Icon name="refresh" :size="13" />{{ t('Refresh saved balance') }}</button>
        <button v-if="mode!=='disabled'" type="button" class="small-button" :disabled="disabled" @click="test"><Icon name="play" :size="13" />{{ t(busy?'Working…':'Test balance script') }}</button>
      </div>
    </footer>
  </section>
</template>
<style scoped>
.balance-editor-head{flex-wrap:wrap}
.balance-fields{min-width:0;margin:0;padding:0;border:0}
.balance-editor-foot{align-items:flex-end;justify-content:space-between;flex-wrap:wrap;gap:12px}
.balance-state{flex-shrink:0;font-size:var(--text-xs);color:var(--muted);white-space:nowrap}
.balance-state.warn{color:var(--warn-ink)}
.balance-state.ok{color:var(--ok-ink)}
.balance-modes{display:inline-flex;margin-left:auto;padding:3px;border-radius:var(--radius-full);background:var(--fill)}
.balance-modes button{height:28px;padding:0 14px;border:0;border-radius:var(--radius-full);background:none;color:var(--ink-soft);font-size:var(--text-sm);cursor:pointer;white-space:nowrap}
.balance-modes button.selected{background:var(--surface);color:var(--ink);font-weight:600;box-shadow:var(--shadow-sm)}
.balance-currency{text-transform:uppercase}
.balance-script-head{display:flex;align-items:center;gap:10px;font-size:var(--text-sm);font-weight:550;color:var(--ink)}
.balance-examples{margin-left:auto;width:auto !important;max-width:55%;min-height:28px;padding:3px 10px;font-size:var(--text-xs)}
/* A balance script is a few lines; the shared editor's height is sized for longer code. */
.balance-script .code-editor :deep(.cm-content),.balance-script .code-editor :deep(.cm-gutter){min-height:96px}
.balance-script .form-help code{padding:0 4px;border-radius:var(--radius-xs);background:var(--fill);color:var(--ink-soft);font-size:.9em}
.balance-api-help{font-size:var(--text-xs);color:var(--muted)}
.balance-api-help summary{cursor:pointer;width:max-content}
.balance-api-help pre{margin:8px 0;padding:10px 12px;border-radius:var(--radius-md);background:var(--code-line);overflow:auto;font:var(--text-xs)/1.6 var(--mono);color:var(--syn-text)}
.balance-outcome{display:grid;gap:8px;min-width:0;flex:1 1 220px}
.balance-preview{padding:10px 12px;border:1px dashed var(--accent-line);border-radius:var(--radius-md);background:var(--accent-soft)}
.balance-preview>strong{display:block;font-size:var(--text-xs);color:var(--accent-ink);margin-bottom:6px}
/* The shared toolbar style never shrinks; here the row wraps inside the card instead of running past it. */
.toolbar-buttons{display:flex;flex-wrap:wrap;justify-content:flex-end;gap:8px;margin-left:auto;flex-shrink:1;min-width:0}
.toolbar-buttons .small-button{display:inline-flex;align-items:center;gap:5px}
.form-help{margin:0}
</style>
