use proto::{Card, CardParseError};
use crate::error::RulesError;

pub fn parse_card(text: &str) -> Result<Card, RulesError> { Ok(text.trim().parse::<Card>()?) }

/// "AsKd", "As Kd" and "As,Kd" are accepted; duplicates are rejected.
pub fn parse_cards(text: &str) -> Result<Vec<Card>, RulesError> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    if chars.len() % 2 != 0 { return Err(CardParseError::Length(text.to_string()).into()); }
    let mut out = Vec::with_capacity(chars.len() / 2);
    for pair in chars.chunks(2) {
        let card: Card = format!("{}{}", pair[0], pair[1]).parse()?;
        if out.contains(&card) { return Err(CardParseError::Duplicate(card).into()); }
        out.push(card);
    }
    Ok(out)
}

pub fn parse_hand(text: &str) -> Result<[Card; 2], RulesError> {
    let v = parse_cards(text)?;
    if v.len() != 2 { return Err(CardParseError::Length(text.to_string()).into()); }
    Ok([v[0], v[1]])
}

pub fn cards_to_string(cards: &[Card]) -> String { cards.iter().map(|c| c.to_string()).collect() }
