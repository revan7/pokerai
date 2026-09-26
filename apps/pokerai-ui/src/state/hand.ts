import type {HandState,Card,Seat,BeginHand} from '../ipc/types.gen';
import type {StackDrafts} from './session';

export function parseCards(text:string):Card[]{
  const input=text.replace(/\s/g,'');if(input.length%2!==0)throw new Error('Enter rank and suit for each card');
  const cards:Card[]=[];
  for(let i=0;i<input.length;i+=2){
    const rank='23456789TJQKA'.indexOf(input.charAt(i).toUpperCase());
    const suit='cdhs'.indexOf(input.charAt(i+1).toLowerCase());
    if(rank<0||suit<0)throw new Error('Cards use ranks 2–9,T,J,Q,K,A and suits c,d,h,s');
    const card='23456789TJQKA'.charAt(rank)+'cdhs'.charAt(suit);if(cards.includes(card))throw new Error('Duplicate card');cards.push(card);
  }
  return cards;
}
export function cardText(card:Card):string{return card;}
export function decisionReason(h:HandState|null):string|null {
  if(h===null)return 'no hand';if(h.phase.phase!=='betting')return 'hand complete or awaiting board';
  const i=h.hero;if(!h.dealt.includes(i))return 'hero is not dealt';
  if(h.derived.folded[i])return 'hero folded';if(h.derived.all_in[i])return 'hero all-in';
  if(h.hero_cards===null||h.hero_cards.length!==2)return 'hero cards unknown';
  if(h.derived.to_act!==h.hero)return 'another seat to act';
  if(h.derived.legal.length<2)return 'fewer than two legal actions';return null;
}
export function prefill(previous:HandState|null,defaults:StackDrafts):StackDrafts {
  const values={...defaults};if(previous===null)return values;
  previous.dealt.forEach((seat,index)=>{
    const start=previous.stacks_start[index],remaining=previous.derived.stacks_remaining[seat];
    // MI-10: `start - (start - remaining)` is identically `remaining`; written this way to match
    // spec §4.3's wording literally ("previous hand's start minus its committed chips") and because
    // both `start`/`remaining` need their own `undefined` guard (dealt-order vs. physical-seat
    // indexing) before `committed` can be computed at all.
    if(start!==undefined&&remaining!==undefined){const committed=start-remaining;values[seat]=start-committed;}
  });return values;
}
// No pot is awarded by the UI. Engine remaining stacks include returned uncalled money.
// The user corrects winners' stacks and rebuys during confirmation on the next N.
export type Wizard={step:'button'|'hero'|'dealt'|'stack'|'cards'|'done';button:Seat|null;hero:Seat|null;
  dealt:Seat[];stacks:StackDrafts;confirmed:Seat[];index:number;text:string;pristine:boolean;cards:[Card,Card]|null};
export function newWizard(dealt:Seat[],stacks:StackDrafts):Wizard {
  return {step:'button',button:null,hero:null,dealt:[...dealt],stacks:{...stacks},confirmed:[],
    index:0,text:'',pristine:true,cards:null};
}
export function wizardKey(w:Wizard,key:string):Wizard {
  if(w.step==='done')return w;
  const digit=/^[1-6]$/.test(key)?Number(key)-1:null;
  if(w.step==='button'&&digit!==null)return {...w,button:digit,step:'hero'};
  if(w.step==='hero'&&digit!==null)return {...w,hero:digit,step:'dealt'};
  if(w.step==='dealt'){
    if(digit!==null)return {...w,dealt:(w.dealt.includes(digit)?w.dealt.filter(s=>s!==digit):[...w.dealt,digit]).sort((a,b)=>a-b)};
    if(key==='Enter'){
      if(w.dealt.length<3||w.dealt.length>6||w.hero===null||w.button===null||!w.dealt.includes(w.hero)||!w.dealt.includes(w.button))throw new Error('Select 3 to 6 dealt seats including button and hero');
      const first=w.dealt[0];if(first===undefined)throw new Error('No dealt seat');
      return {...w,step:'stack',index:0,text:String(w.stacks[first]??0),confirmed:[],pristine:true};
    }return w;
  }
  if(w.step==='stack'){
    const seat=w.dealt[w.index];if(seat===undefined)throw new Error('Missing stack seat');
    if(key==='Enter'){
      const n=Number(w.text);if(!/^\d+$/.test(w.text)||!Number.isInteger(n)||n>2_147_483_647)throw new Error('Stack must be whole chips');
      const confirmed=[...w.confirmed,seat],stacks={...w.stacks,[seat]:n},next=w.dealt[w.index+1];
      return next===undefined?{...w,stacks,confirmed,step:'cards',text:'',pristine:true}:
        {...w,stacks,confirmed,index:w.index+1,text:String(stacks[next]??0),pristine:true};
    }
    if(/^\d$/.test(key))return {...w,text:(w.pristine?'':w.text)+key,pristine:false};
    if(key==='Backspace')return {...w,text:w.text.slice(0,-1),pristine:false};return w;
  }
  if(w.step==='cards'){
    if(key==='Backspace')return {...w,text:w.text.slice(0,-1)};
    if(key==='Enter'){
      if(w.text==='')return {...w,step:'done',cards:null};
      const cards=parseCards(w.text);const a=cards[0],b=cards[1];
      if(cards.length!==2||a===undefined||b===undefined)throw new Error('Enter two hero cards');
      return {...w,step:'done',cards:[a,b]};
    }
    if(/^[2-9tjqkacdh s]$/i.test(key)&&key!==' '&&w.text.length<4)return {...w,text:w.text+key};
  }return w;
}
export function wizardResult(w:Wizard):BeginHand|null{
  if(w.step!=='done'||w.hero===null||w.button===null||w.confirmed.length!==w.dealt.length)return null;
  return {button:w.button,hero:w.hero,dealt:w.dealt,stacks:w.dealt.map(s=>w.stacks[s]??0),hero_cards:w.cards};
}
