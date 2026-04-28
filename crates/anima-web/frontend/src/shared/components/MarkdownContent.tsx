import { memo } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { PrismLight as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';

import bash from 'refractor/bash';
import c from 'refractor/c';
import cpp from 'refractor/cpp';
import css from 'refractor/css';
import diff from 'refractor/diff';
import go from 'refractor/go';
import java from 'refractor/java';
import javascript from 'refractor/javascript';
import json from 'refractor/json';
import markdown from 'refractor/markdown';
import markup from 'refractor/markup';
import python from 'refractor/python';
import rust from 'refractor/rust';
import sql from 'refractor/sql';
import toml from 'refractor/toml';
import typescript from 'refractor/typescript';
import yaml from 'refractor/yaml';

for (const [name, lang] of Object.entries({
  bash, c, cpp, css, diff, go, java, javascript,
  json, markdown, html: markup, python, rust, sql, toml, typescript, yaml,
})) {
  SyntaxHighlighter.registerLanguage(name, lang);
}

const remarkPlugins = [remarkGfm];

function CodeBlock({ className, children, ...rest }: React.ComponentProps<'code'>) {
  const match = /language-(\w+)/.exec(className || '');
  const text = String(children).replace(/\n$/, '');
  if (!match) return <code className={className} {...rest}>{children}</code>;
  return (
    <SyntaxHighlighter style={oneDark} language={match[1]} PreTag="div">
      {text}
    </SyntaxHighlighter>
  );
}

const components = {
  code: CodeBlock,
  a: ({ children, ...props }: React.ComponentProps<'a'>) => (
    <a {...props} target="_blank" rel="noopener noreferrer">{children}</a>
  ),
};

export const MarkdownContent = memo(function MarkdownContent({ content }: { content: string }) {
  return (
    <div className="chat-markdown">
      <Markdown remarkPlugins={remarkPlugins} components={components}>
        {content}
      </Markdown>
    </div>
  );
});
