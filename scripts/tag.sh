#!/bin/bash

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}🏷️  开始获取最新标签...${NC}"

# 获取最新标签
git fetch --tags

# 如果没有标签，返回 v0.0.0 作为兜底
latest_tag=$(git describe --tags `git rev-list --tags --max-count=1` 2>/dev/null || echo "v0.0.0")
echo -e "${YELLOW}📋 Latest tag: ${latest_tag}${NC}"

# 解析版本号
version=${latest_tag#v}
IFS='.' read -r -a parts <<<"$version"
last_idx=$((${#parts[@]} - 1))
parts[$last_idx]=$((${parts[$last_idx]} + 1))
new_version=$(IFS='.'; echo "${parts[*]}")
new_tag="v$new_version"

echo -e "${GREEN}🎯 New tag: ${new_tag}${NC}"

# 生成提交记录清单
echo -e "${BLUE}📝 生成提交记录清单...${NC}"

# 获取从上一个标签到当前HEAD的提交记录
if [ "$latest_tag" = "v0.0.0" ]; then
    # 如果没有之前的标签，获取所有提交
    commit_range="HEAD"
    echo -e "${YELLOW}💡 没有找到之前的标签，将包含所有提交记录${NC}"
else
    # 从上一个标签到当前HEAD的提交
    commit_range="${latest_tag}..HEAD"
    echo -e "${YELLOW}📊 获取从 ${latest_tag} 到当前的提交记录${NC}"
fi

# 生成提交记录清单，格式：- [commit_hash] commit_message
commit_log=$(git log $commit_range --pretty=format:"- [%h] %s" --reverse)

if [ -z "$commit_log" ]; then
    echo -e "${YELLOW}⚠️  没有找到新的提交记录${NC}"
    tag_message="Release ${new_tag}"
else
    echo -e "${GREEN}📋 提交记录清单:${NC}"
    echo "$commit_log"
    echo ""

    # 构建标签消息
    tag_message="Release ${new_tag}

## Changes since ${latest_tag}

$commit_log"
fi

# 确认创建标签
echo -e -n "${YELLOW}确认创建标签 ${new_tag}? (y/n): ${NC}"
read confirm

if [ "$confirm" = "y" ] || [ "$confirm" = "Y" ]; then
    echo -e "${BLUE}🚀 创建带描述的标签 ${new_tag}...${NC}"

    # 使用 -a 参数创建带注释的标签，-m 参数添加消息
    git tag -a $new_tag -m "$tag_message"

    echo -e "${BLUE}📤 推送标签到远程仓库...${NC}"
    git push origin $new_tag

    echo -e "${GREEN}✅ 标签 ${new_tag} 创建并推送成功！${NC}"
    echo -e "${GREEN}📄 标签描述已包含 $(echo "$commit_log" | wc -l | tr -d ' ') 个提交记录${NC}"
else
    echo -e "${RED}❌ 标签创建已取消${NC}"
fi
