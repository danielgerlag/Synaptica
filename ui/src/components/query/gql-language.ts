import { StreamLanguage, StringStream } from '@codemirror/language'

const keywords = new Set([
  'MATCH', 'WHERE', 'RETURN', 'INSERT', 'DELETE', 'SET', 'REMOVE',
  'CREATE', 'DROP', 'OPTIONAL', 'WITH', 'UNION', 'ORDER', 'BY',
  'ASC', 'DESC', 'LIMIT', 'OFFSET', 'DISTINCT', 'AS', 'AND', 'OR',
  'NOT', 'XOR', 'IN', 'IS', 'NULL', 'TRUE', 'FALSE', 'CASE', 'WHEN',
  'THEN', 'ELSE', 'END', 'EXISTS',
])

const functions = new Set([
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'COLLECT',
  'TOSTRING', 'TOINTEGER', 'TOFLOAT', 'SIZE',
])

export const gqlLanguage = StreamLanguage.define({
  token(stream: StringStream): string | null {
    // Whitespace
    if (stream.eatSpace()) return null

    // Line comment
    if (stream.match('//')) {
      stream.skipToEnd()
      return 'comment'
    }

    // Single-quoted string
    if (stream.match("'")) {
      while (!stream.eol()) {
        if (stream.next() === "'") return 'string'
      }
      return 'string'
    }

    // Parameter
    if (stream.match(/^\$[a-zA-Z_]\w*/)) return 'variableName.special'

    // Number
    if (stream.match(/^\d+(\.\d+)?/)) return 'number'

    // Word (keyword, function, or identifier)
    if (stream.match(/^[a-zA-Z_]\w*/)) {
      const word = stream.current().toUpperCase()
      if (keywords.has(word)) return 'keyword'
      if (functions.has(word)) return 'function'
      return 'variableName'
    }

    // Operators and punctuation
    stream.next()
    return 'punctuation'
  },
})
