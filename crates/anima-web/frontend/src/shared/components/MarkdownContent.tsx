import { memo } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';

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
