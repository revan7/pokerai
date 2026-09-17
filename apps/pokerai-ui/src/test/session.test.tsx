import { expect,test } from 'vitest';
import { render,screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ConfigScreen } from '../components/ConfigScreen';
import { readSession } from '../state/session';
import { config } from './fixtures';
import { FakeBackend } from './fakeBackend';
import { installMockIpc } from './mockIpc';
import { tauriBackend } from '../ipc/backend';
test('session_settings_validate_and_use_engine_revision',async()=>{
  const b=new FakeBackend();installMockIpc(b);const errors:unknown[]=[];
  render(<ConfigScreen config={config} stacks={{}} activeHand={true}
    save={async(c)=>{await tauriBackend.set_game_config(c);}} onError={e=>errors.push(e)}/>);
  expect(screen.getByText('Changes apply to the next hand.')).toBeVisible();
  const user=userEvent.setup();const budget=screen.getByLabelText('Flop budget (seconds)');
  expect(budget).toHaveValue(10);
  await user.clear(screen.getByLabelText('Rake cap (chips)'));
  await user.type(screen.getByLabelText('Rake cap (chips)'),'2.5');
  await user.click(screen.getByRole('button',{name:'Save session'}));
  expect(b.calls[0]).toEqual(['set_game_config',{config:{...config,
    rake:{kind:'pot_rake',rate:.05,cap_mchips:2500,no_flop_no_drop:true}}}]);
  const configForm=screen.getByTestId('config-form');
  if(!(configForm instanceof HTMLFormElement)) throw new Error('config-form is not a form element');
  const form=new FormData(configForm);
  form.set('flop_budget_s','31');expect(()=>readSession(form,config)).toThrow('1 to 30');
  expect(errors).toHaveLength(0);
});

test('session_input_edge_cases',()=>{
  const form=(fields:Record<string,string|string[]>)=>{
    const f=new FormData();
    for(const [k,v] of Object.entries(fields)) for(const value of Array.isArray(v)?v:[v]) f.append(k,value);
    return f;
  };
  const base={chip_label:'$1',sb_chips:'5',bb_chips:'10',straddle:'',
    seat:['0','1','2','3','4','5'],flop_budget_s:'10',rate:'5',cap:'5',rake:'PotRake'};
  const time=readSession(form({...base,rake:'TimeCharge'}),config);
  expect(time.config.rake).toEqual({kind:'time_charge'});
  const fiveSeats=form({...base,seat:['0','1','2','3','4'],straddle:'20'});
  expect(()=>readSession(fiveSeats,config)).toThrow('six seats');
  const belowTwiceBb=form({...base,straddle:'15'}); // bb=10, so 2*bb=20; six seats present
  expect(()=>readSession(belowTwiceBb,config)).toThrow('six seats');
  const capped=readSession(form({...base,cap:'2.501'}),config);
  expect(capped.config.rake).toEqual({kind:'pot_rake',rate:.05,cap_mchips:2501,no_flop_no_drop:false});
});
